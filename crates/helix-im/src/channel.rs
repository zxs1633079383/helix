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

use crate::error::ImError;
use crate::state::{ChannelId, Cursor, InflightSync, Seq};
use crate::sync_session::EventEnvelope;
use helix_core::effect::{StorageOp, TimerId};
use helix_core::{Correlation, Effect, EffectSink};
use std::collections::BTreeMap;

/// E6：单 channel gate 乱序缓冲的客户端容量上限。
///
/// 取值须 > 合法最大缺口（承压实测 5000 条乱序堆积由 gate 正常回填），
/// 又须 << OOM 阈值。超限不再无界堆积、不再只依赖服务端 too_long——
/// 主动走 too_long 恢复路径（清 buffer + 通知 UI 全量刷新 + sync 重拉）。
const MAX_GATE_BUFFER: usize = 16_384;

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
    /// B1 gate_inflight：同 channel 同时只允许一个在途 sync Http。
    ///
    /// Some(InflightSync(corr)) 表示有在途 sync；续拉发出时置 Some，PortReply 到达后清 None。
    /// 防止 needs_continuation 并发自踩（连续触发多个相同 channel 的 sync）。
    pub inflight_sync: Option<InflightSync>,
    /// E3 续拉：记录发出续拉 sync 时的 fromSeq（上一批请求的起点）
    ///
    /// 终止保证：若 persist ok 后 cursor == sync_from_seq（cursor 未前进），
    /// 停止续拉 + log warn（意味着服务端返回的所有事件都低于 fromSeq，幂等丢弃）。
    pub last_sync_from_seq: Option<Seq>,
}

impl Channel {
    pub fn new(id: ChannelId, initial_cursor: u64) -> Self {
        Self {
            id,
            cursor: Cursor::new(Seq(initial_cursor)),
            gate: None,
            pending_commits: std::collections::HashMap::new(),
            buffer: BTreeMap::new(),
            inflight_sync: None,
            last_sync_from_seq: None,
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
            std::cmp::Ordering::Equal => self.apply_and_flush_contiguous(ev, fx, now_ms),
            std::cmp::Ordering::Less => Ok(()), // dup drop，幂等
            std::cmp::Ordering::Greater => self.buffer_and_arm_gate(ev, fx),
        }
    }

    /// 处理 PortReply{corr=X, Ok}：推进 cursor（写成功才推）
    ///
    /// sync 权威路径的 commit 阶段：
    /// 1. 从 pending_commits 取出 target_seq
    /// 2. commit_cursor（MAX guard）
    /// 3. flush_contiguous_from_buffer（尝试继续处理缓冲中已就绪的事件）
    pub fn on_persist_ok(&mut self, corr: Correlation, fx: &mut EffectSink) -> Result<(), ImError> {
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

        // 吐 PersistFire（幂等落库：单条 upsert message 表，ON CONFLICT(temporary_id) DO UPDATE）
        // WS 路径用 PersistFire（fire-and-forget），不需要 PortReply 回报
        fx.push(Effect::PersistFire {
            ops: vec![event_to_upsert_op(&ev)],
        });

        // 推进内存 cursor（WS 路径：直接 try_advance，不等 PortReply）
        // 理由：WS 顺序可靠，PersistFire 幂等（写 678 message 表，与 sync 路径同表同 PK）；
        // 异常落库失败时由 sync 路径从 cursor 重拉同事件覆盖恢复（两路径表名一致，恢复闭环成立）
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
    fn buffer_and_arm_gate(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
    ) -> Result<(), ImError> {
        // E6：客户端 buffer 容量安全网。缺口过大（远超 gate 正常回填规模）时，
        // 不再无界堆积、不再只等服务端 too_long——主动走 too_long 恢复：
        // 清 buffer + 通知 UI 全量刷新 + 保持 gate（cursor 有效不回退，sync 从
        // cursor+1 重拉缺口）。丢弃的 far-future 事件由后续 sync/WS 重新投递。
        // 退化场景（永久缺口 + 持续洪峰）每满一次触发一次恢复，触发频率有界。
        if self.buffer.len() >= MAX_GATE_BUFFER {
            self.buffer.clear();
            fx.push(crate::acl::to_effect::emit_sync_too_long(self.id));
            self.arm_gate_if_needed(fx);
            return Ok(());
        }

        self.buffer.insert(ev.seq, ev);
        self.arm_gate_if_needed(fx);
        Ok(())
    }

    /// arm 1s gate timer（若尚未 armed）。
    ///
    /// BLK-1b：TimerId 由 channel_id 偏移 + cursor 低位派生，避免多 channel timer 碰撞。
    fn arm_gate_if_needed(&mut self, fx: &mut EffectSink) {
        if self.gate.is_none() {
            let timer_id = TimerId::from_raw(
                self.id
                    .hash_u64()
                    .wrapping_mul(1000)
                    .wrapping_add(self.cursor.value().0 & 0xFFF),
            );
            fx.push(Effect::ScheduleTimer {
                id: timer_id,
                after_ms: 1_000,
            });
            self.gate = Some(Gate {
                timer_id,
                expected_seq: Seq(self.cursor.value().0 + 1),
            });
        }
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
        self.buffer
            .range(..=target_seq)
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

/// 将 EventEnvelope 转为落库 StorageOp（批量 upsert，ON CONFLICT(temporary_id) DO UPDATE）。
///
/// ## 唯一落库映射（sync 路径与 WS 推送路径共用此函数）
///
/// 写向 678 真库 `message` 表（PK=temporary_id），与发送态完全同表同主键语义——
/// 不再有 "messages" 影子表。两条 live 写路径（WS 推送 / B-rest 增量）经此函数收口，
/// 收到的事件真正落到磁盘 message 表。
///
/// ## 字段映射（真源 = cses-client `From<types::Post> for Message`）
///
/// - `temporary_id`（PK）：`temporary_id` ?? `id` ?? `evt:{channel}:{seq}` 合成键。
///   三级回退保证 PK 恒非空、且对同一事件**确定性幂等**（re-sync 同批 = 同 PK 覆盖 = O(1)，
///   不产生重复行、不引发 sync 风暴）。
/// - `id` / `channel_id` / `user_id` / `type` / `message` / `props` / `create_at`：从事件
///   `raw`（ChannelEvent wire JSON）提取，缺省安全回退（边界零信任，无 unwrap）。
/// - `props`：保留服务端原始 props 对象（业务扩展）；用于已读位 / event_seq 等下游对账。
///
/// 热路径：单条 INSERT ... ON CONFLICT(temporary_id) DO UPDATE = O(1)，无 read-modify-write。
pub fn event_to_upsert_op(ev: &EventEnvelope) -> StorageOp {
    use helix_core::effect::{SqlValue, UpsertSpec};

    // raw 是 ChannelEvent wire JSON（parser 产出，边界零信任）。post 字段可能在顶层，
    // 也可能嵌套在 "post" / "data" 子对象下——按优先级探查，全程安全回退不 panic。
    let root: serde_json::Value =
        serde_json::from_slice(ev.raw.as_ref()).unwrap_or(serde_json::Value::Null);
    let post = root
        .get("post")
        .or_else(|| root.get("data"))
        .unwrap_or(&root);

    // 取字符串字段：先 post 视图，回退 root 顶层（兼容扁平 wire）。
    let pick_str = |key: &str| -> Option<String> {
        post.get(key)
            .and_then(|v| v.as_str())
            .or_else(|| root.get(key).and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };

    // 兼容 snake_case / camelCase（Go wire 多为 camelCase；既有测试帧用 snake_case）。
    let temporary_id = pick_str("temporary_id").or_else(|| pick_str("temporaryId"));
    let id = pick_str("id").unwrap_or_default();
    let channel_id = pick_str("channel_id")
        .or_else(|| pick_str("channelId"))
        // 回退到 envelope 权威 channel_id（parser 已校验）。
        .unwrap_or_else(|| ev.channel_id.as_str().to_string());
    let user_id = pick_str("user_id")
        .or_else(|| pick_str("userId"))
        .unwrap_or_default();
    let msg_type = pick_str("type")
        .or_else(|| pick_str("post_type"))
        .unwrap_or_default();
    let message = pick_str("message").unwrap_or_default();
    let create_at = post
        .get("create_at")
        .or_else(|| post.get("createAt"))
        .or_else(|| root.get("create_at"))
        .or_else(|| root.get("createAt"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    // props 保留服务端原始对象（业务扩展 / event_seq / read map 下游对账）。
    let props = post
        .get("props")
        .or_else(|| root.get("props"))
        .map(|v| v.to_string())
        .unwrap_or_default();

    // PK 三级回退（真源 cses-client：temporary_id ?? id；再加 evt:{ch}:{seq} 合成兜底，
    // 保证收到的事件即便无 temporary_id 也有确定性非空 PK，re-sync 幂等覆盖同行）。
    let pk = temporary_id
        .filter(|s| !s.is_empty())
        .or_else(|| {
            if id.is_empty() {
                None
            } else {
                Some(id.clone())
            }
        })
        .unwrap_or_else(|| format!("evt:{}:{}", ev.channel_id.as_str(), ev.seq.0));

    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![vec![
            ("temporary_id".to_string(), SqlValue::Text(pk)),
            ("id".to_string(), SqlValue::Text(id)),
            ("channel_id".to_string(), SqlValue::Text(channel_id)),
            ("user_id".to_string(), SqlValue::Text(user_id)),
            ("type".to_string(), SqlValue::Text(msg_type)),
            ("message".to_string(), SqlValue::Text(message)),
            ("props".to_string(), SqlValue::Text(props)),
            ("create_at".to_string(), SqlValue::Integer(create_at)),
        ]],
        conflict_key: Some("temporary_id"),
    })
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{ChannelId, Seq};
    use crate::sync_session::{EventEnvelope, EventKind};
    use helix_core::{Effect, EffectSink};

    fn gapped_ev(ch: u64, seq: u64) -> EventEnvelope {
        EventEnvelope::new(
            ChannelId::from_test_u64(ch),
            Seq(seq),
            EventKind::PostUpsert,
            bytes::Bytes::from_static(b"{}"),
        )
    }

    fn is_too_long_emit(e: &Effect) -> bool {
        matches!(e, Effect::Emit { event }
            if String::from_utf8_lossy(event.0.as_ref()).contains("im:sync:too_long"))
    }

    /// 回归锚: E6 —— gate buffer 客户端上限。
    ///
    /// 制造永久缺口（全部 seq >= 2，缺 seq=1），持续灌入远超 MAX_GATE_BUFFER 的
    /// 乱序事件。断言：① buffer 始终有界（永不超过上限）；② 越限时主动
    /// emit im:sync:too_long；③ cursor 不被回退（安全网靠 sync 从 cursor+1 重拉）。
    #[test]
    fn test_gate_buffer_capped_proactive_too_long_e6() {
        let mut ch = Channel::new(ChannelId::from_test_u64(7), 0); // cursor=0 → expected=1，制造永久缺口
        let mut fx = EffectSink::new();
        let mut saw_too_long = false;

        // 灌到越过上限 + 余量，确保触发至少一次主动恢复
        for seq in 2..(MAX_GATE_BUFFER as u64 + 2 + 128) {
            fx.clear();
            ch.ingest(gapped_ev(7, seq), &mut fx, 1_000)
                .expect("ingest gapped event");
            assert!(
                ch.buffer.len() <= MAX_GATE_BUFFER,
                "buffer 必须有界（≤ 上限），实际 {}",
                ch.buffer.len()
            );
            if fx.as_slice().iter().any(is_too_long_emit) {
                saw_too_long = true;
            }
        }

        assert!(
            saw_too_long,
            "越过 buffer 上限必须主动 emit im:sync:too_long（不再静等服务端）"
        );
        assert_eq!(
            ch.cursor.value(),
            Seq(0),
            "E6 安全网不回退 cursor（缺口由后续 sync 从 cursor+1 重拉）"
        );
    }

    /// 上限以内正常乱序堆积不误触发 too_long（不误伤承压合法 5000 缺口场景）。
    #[test]
    fn test_gate_buffer_under_cap_no_false_too_long_e6() {
        let mut ch = Channel::new(ChannelId::from_test_u64(3), 0);
        let mut fx = EffectSink::new();

        for seq in 2..=5001 {
            // 承压同款 5000 缺口
            ch.ingest(gapped_ev(3, seq), &mut fx, 1_000)
                .expect("ingest");
        }
        assert_eq!(ch.buffer.len(), 5000, "5000 缺口应全部正常堆积");
        assert!(
            !fx.as_slice().iter().any(is_too_long_emit),
            "上限以内不得误触发 too_long"
        );
    }
}
