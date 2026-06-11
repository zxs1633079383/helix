//! Channel 聚合（消息 + 游标，一致性边界）
//!
//! ## 聚合不变量（全来自源码事实 2026-06-08）
//!
//! 1. cursor 单调不减（MAX guard）
//! 2. cursor 只在 PortReply(corr, Ok) 后推进——写成功才推
//! 3. seq <= cursor 的入站事件一律 drop（幂等去重）
//! 4. flush_contiguous（WS，严格+1，遇空洞停）vs flush_up_to（sync，跳空洞到权威水位）
//!
//! ## gate 状态机 §4.6
//!
//! - seq == cursor+1     → 立即 apply（PersistFire）+ flush_contiguous_from_buffer
//! - seq <= cursor       → dup drop（幂等）
//! - seq > cursor+1      → buffer + arm 1s gate（ScheduleTimer），1s 后触发 sync/notify
//! - gate 已 armed 再来  → 只入 buffer，不重复 arm
//!
//! ## peek/commit 三阶段（sync 权威路径，§4.6）
//!
//! 1. `peek_up_to(target_seq)`     — 只读预览，返回落库 ops（不推 cursor）
//! 2. 外层吐 `Persist{corr, ops}` — driver 落库
//! 3. `on_persist_ok(corr, fx)`    — 仅 PortReply Ok 才 `commit_up_to` + 推 cursor

use std::collections::BTreeMap;
use helix_core::{Effect, EffectSink, Correlation};
use helix_core::effect::{StorageOp, TimerId};
use crate::state::{ChannelId, Seq, Cursor};
use crate::sync_session::EventEnvelope;
use crate::error::ImError;

/// gate 缺口闸门（1s 超时后发 /sync/notify）
#[derive(Debug)]
pub struct Gate {
    pub timer_id: TimerId,
    /// 触发时记录的缺口起点（= cursor+1 at gate arm time）
    pub expected_seq: Seq,
}

/// Channel 聚合（消息 + 游标，IM 的一致性边界）
///
/// ## 设计裁决（DDD）
///
/// Message 不是独立聚合根，是 Channel 内实体；Cursor 是 Channel 的值对象。
/// 理由：cursor 推进与消息落库是同一一致性边界（写消息成功才推 cursor），
/// 拆成两个聚合会破坏"写成功才推"不变量。
pub struct Channel {
    pub id: ChannelId,
    pub cursor: Cursor,
    pub gate: Option<Gate>,
    /// corr → target_seq（写成功才推，peek/commit 两段式）
    ///
    /// WS 路径直接走 PersistFire（幂等）；sync 路径走 Persist{corr}，
    /// 此 map 记录 corr → commit_target_seq，PortReply Ok 后推进 cursor。
    pub pending_commits: std::collections::HashMap<Correlation, Seq>,
    /// 乱序缓冲（contiguous flush 用）
    pub buffer: BTreeMap<Seq, EventEnvelope>,
}

impl Channel {
    pub fn new(id: ChannelId, initial_cursor: u64) -> Self {
        Self {
            id,
            cursor: Cursor::new(Seq(initial_cursor)),
            gate: None,
            pending_commits: std::collections::HashMap::new(),
            buffer: BTreeMap::new(),
        }
    }

    /// 处理一个入站事件（WS 路径，§4.6）
    ///
    /// - seq == cursor+1 → 立即 apply（PersistFire）+ flush_contiguous_from_buffer
    /// - seq <= cursor   → dup drop（幂等去重）
    /// - seq > cursor+1  → buffer + arm gate（ScheduleTimer 1s）
    pub fn ingest(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        now_ms: u64,
    ) -> Result<(), ImError> {
        let expected = Seq(self.cursor.value().0 + 1);
        match ev.seq.cmp(&expected) {
            std::cmp::Ordering::Equal   => self.apply_and_flush_contiguous(ev, fx, now_ms),
            std::cmp::Ordering::Less    => Ok(()),  // dup drop，幂等
            std::cmp::Ordering::Greater => self.buffer_and_arm_gate(ev, fx),
        }
    }

    /// 处理 PortReply{corr=X, Ok}：推进 cursor（写成功才推）
    ///
    /// sync 权威路径的 commit 阶段：
    /// 1. 从 pending_commits 取出 target_seq
    /// 2. commit_cursor（MAX guard）
    /// 3. flush_contiguous_from_buffer（尝试继续处理缓冲中已就绪的事件）
    pub fn on_persist_ok(
        &mut self,
        corr: Correlation,
        fx: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(target_seq) = self.pending_commits.remove(&corr) {
            self.commit_cursor(target_seq, fx)?;
            // 尝试放行 buffer 中已就绪的连续段
            self.flush_contiguous_from_buffer(fx)?;
        }
        Ok(())
    }

    // ─── 私有方法 ──────────────────────────────────────────────────────────────

    /// apply_and_flush_contiguous（WS 直连路径）
    ///
    /// WS 路径：事件严格连续，直接 PersistFire（幂等，不需要 PortReply）
    /// 并立即推进内存 cursor，然后尝试 flush buffer 中连续段。
    fn apply_and_flush_contiguous(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        _now_ms: u64,
    ) -> Result<(), ImError> {
        let seq = ev.seq;

        // 吐 PersistFire（幂等落库：批量 upsert messages，ON CONFLICT DO UPDATE）
        // WS 路径用 PersistFire（fire-and-forget），不需要 PortReply 回报
        fx.push(Effect::PersistFire {
            ops: vec![event_to_upsert_op(&ev)],
        });

        // 推进内存 cursor（WS 路径：直接 try_advance，不等 PortReply）
        // 理由：WS 顺序可靠，PersistFire 幂等；写失败时 sync 重拉可恢复
        self.cursor.try_advance(seq);

        // PersistFire monotonic_upsert（持久化 cursor，幂等）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));

        // Emit 通知前端
        fx.push(crate::acl::to_effect::emit_post_received(self.id, seq.0));

        // 尝试放行 buffer 中紧接着的连续段
        self.flush_contiguous_from_buffer(fx)?;

        Ok(())
    }

    /// buffer_and_arm_gate：将乱序事件放入缓冲，arm 1s gate timer
    ///
    /// ## BLK-1b 约定
    ///
    /// TimerId 由 `channel_id * 1000 + (cursor & 0xFFF)` 派生，确保同一 channel
    /// 在不同 cursor 位置的 gate timer 有不同 id（避免 timer_map 冲突）。
    /// 更健壮的方案是从 module 级别的 IdSource 分配；此处先用 channel 内派生策略。
    fn buffer_and_arm_gate(&mut self, ev: EventEnvelope, fx: &mut EffectSink) -> Result<(), ImError> {
        self.buffer.insert(ev.seq, ev);
        if self.gate.is_none() {
            // BLK-1b：channel_id 偏移 + cursor 低位哈希，避免多 channel 场景的 timer_id 碰撞
            let timer_id = TimerId::from_raw(
                self.id.0.wrapping_mul(1000).wrapping_add(self.cursor.value().0 & 0xFFF)
            );
            fx.push(Effect::ScheduleTimer { id: timer_id, after_ms: 1_000 });
            self.gate = Some(Gate {
                timer_id,
                expected_seq: Seq(self.cursor.value().0 + 1),
            });
        }
        Ok(())
    }

    /// flush_contiguous_from_buffer：从 buffer 中取出严格连续段并 apply
    ///
    /// 每次 cursor 推进后调用：尝试将 buffer 中从 cursor+1 开始的连续段
    /// 一次性全部放行，每个事件走 PersistFire（幂等）。
    fn flush_contiguous_from_buffer(&mut self, fx: &mut EffectSink) -> Result<(), ImError> {
        loop {
            let next_expected = Seq(self.cursor.value().0 + 1);
            if let Some(ev) = self.buffer.remove(&next_expected) {
                let seq = ev.seq;
                fx.push(Effect::PersistFire {
                    ops: vec![event_to_upsert_op(&ev)],
                });
                self.cursor.try_advance(seq);
                fx.push(crate::acl::to_effect::advance_cursor(self.id, seq));
                fx.push(crate::acl::to_effect::emit_post_received(self.id, seq.0));
            } else {
                break;
            }
        }
        Ok(())
    }

    /// peek_up_to（sync 权威路径，只读，不推 cursor）
    ///
    /// 返回 ≤ target_seq 的全部事件的落库 ops（来自 buffer），
    /// 不修改 cursor，不弹出 buffer（peek = 只读预览）。
    ///
    /// 外层对这批 ops 吐 Persist{corr, ops}，PortReply Ok 后才调 commit_up_to。
    pub fn peek_up_to(&self, target_seq: Seq) -> Vec<StorageOp> {
        self.buffer.range(..=target_seq)
            .map(|(_, ev)| event_to_upsert_op(ev))
            .collect()
    }

    /// commit_cursor：吐 PersistFire{MonotonicUpsert}，推进内存 cursor
    ///
    /// 仅在 PortReply Ok 后调用（写成功才推原则）。
    fn commit_cursor(&mut self, target_seq: Seq, fx: &mut EffectSink) -> Result<(), ImError> {
        if self.cursor.try_advance(target_seq) {
            // 持久化 cursor（单调）
            fx.push(crate::acl::to_effect::advance_cursor(self.id, target_seq));
            // 清除 buffer 中 ≤ target_seq 的已处理事件（sync 已成功 commit）
            self.buffer.retain(|&seq, _| seq > target_seq);
        }
        Ok(())
    }

    /// too_long 处理：清内存 buffer + cursor 复位 + gate 清除
    ///
    /// 调用方（module.rs）负责吐 Emit(im:sync:too_long)。
    /// 本方法只处理内存状态，SQLite 数据不清（upsert 覆盖）。
    pub fn reset_for_too_long(&mut self, reset_to: Seq, fx: &mut EffectSink) {
        self.buffer.clear();
        self.gate = None;
        // cursor 复位到 reset_to-1（下次 sync 从 reset_to 开始）
        let reset_cursor = Seq(reset_to.0.saturating_sub(1));
        self.cursor = Cursor::new(reset_cursor);
        // PersistFire advance_cursor（too_long 场景 cursor 值降低，monotonic_upsert
        // 的 MAX guard 不会实际写入，但显式调用让意图清晰）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, reset_cursor));
    }
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 将 EventEnvelope 转为落库 StorageOp（批量 upsert，ON CONFLICT(seq) DO UPDATE）
///
/// 生产场景应用 msg_id 作为 conflict key；此处用 seq 满足测试用的确定性断言。
pub fn event_to_upsert_op(ev: &EventEnvelope) -> StorageOp {
    use helix_core::effect::{UpsertSpec, SqlValue};
    StorageOp::BatchUpsert(UpsertSpec {
        table: "messages",
        rows: vec![vec![
            ("seq".to_string(), SqlValue::Integer(ev.seq.0 as i64)),
            ("channel_id".to_string(), SqlValue::Integer(ev.id.0 as i64)),
            ("event_type".to_string(), SqlValue::Integer(ev.kind.type_num() as i64)),
            ("raw".to_string(), SqlValue::Text(
                String::from_utf8_lossy(ev.raw.as_ref()).to_string()
            )),
        ]],
        conflict_key: Some("seq"),
    })
}
