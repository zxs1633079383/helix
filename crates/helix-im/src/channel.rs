//! Channel 聚合（消息 + 游标，一致性边界）
//!
//! ## 聚合不变量（全来自源码事实 2026-06-08）
//!
//! 1. cursor 单调不减（MAX guard）
//! 2. cursor 只在 PortReply(corr, Ok) 后推进——写成功才推
//! 3. seq <= cursor 的入站事件一律 drop（幂等去重）
//! 4. flush_contiguous（WS，严格+1，遇空洞停）vs flush_up_to（sync，跳空洞到权威水位）

use std::collections::BTreeMap;
use helix_core::{Effect, EffectSink, Correlation, CoreError};
use crate::state::{ChannelId, Seq, Cursor};
use crate::sync_session::EventEnvelope;
use crate::error::ImError;

/// gate 缺口闸门（1s 超时后发 /sync/notify）
#[derive(Debug)]
pub struct Gate {
    pub timer_id: helix_core::TimerId,
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
    pub pending_commits: std::collections::HashMap<Correlation, Seq>,
    /// 乱序缓冲（contiguous flush 用）
    pub buffer: BTreeMap<Seq, EventEnvelope>,
}

impl Channel {
    pub fn new(id: ChannelId, initial_cursor: Seq) -> Self {
        Self {
            id,
            cursor: Cursor::new(initial_cursor),
            gate: None,
            pending_commits: std::collections::HashMap::new(),
            buffer: BTreeMap::new(),
        }
    }

    /// 处理一个入站事件（WS 路径）
    ///
    /// - seq == cursor+1 → 立即 apply + flush_contiguous
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
    pub fn on_persist_ok(
        &mut self,
        corr: Correlation,
        fx: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some(target_seq) = self.pending_commits.remove(&corr) {
            self.commit_cursor(target_seq, fx)?;
            // 触发 buffer 中已就绪的事件
            self.flush_contiguous_from_buffer(fx)?;
        }
        Ok(())
    }

    /// commit_up_to：吐 PersistFire{MonotonicUpsert}，推进内存 cursor（cursor 是 IM 概念）
    fn commit_cursor(&mut self, target_seq: Seq, fx: &mut EffectSink) -> Result<(), ImError> {
        use helix_core::effect::{StorageOp, MonotonicUpsertSpec};
        if self.cursor.try_advance(target_seq) {
            fx.push(Effect::PersistFire {
                ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                    scope_key: self.id.0.to_string(),  // ACL-1 提供 channel_id 值
                    value: target_seq.0 as i64,
                })],
            });
        }
        Ok(())
    }

    fn apply_and_flush_contiguous(
        &mut self,
        ev: EventEnvelope,
        fx: &mut EffectSink,
        _now_ms: u64,
    ) -> Result<(), ImError> {
        // TODO: 分桶（type1/2/3/6），吐 Persist{corr, BatchUpsert}
        // 暂存 pending_commit（写成功才推 cursor）
        // 见架构文档 §4.4 两段式 step 流程
        let _ = ev;
        let _ = fx;
        Ok(())
    }

    fn buffer_and_arm_gate(&mut self, ev: EventEnvelope, fx: &mut EffectSink) -> Result<(), ImError> {
        self.buffer.insert(ev.seq, ev);
        if self.gate.is_none() {
            // TODO: 从 IdSource 分配 TimerId（MIN-1 修复：使用 from_raw，不直接构造）
            let timer_id = helix_core::TimerId::from_raw(self.id.0); // placeholder
            fx.push(Effect::ScheduleTimer { id: timer_id, after_ms: 1_000 });
            self.gate = Some(Gate {
                timer_id,
                expected_seq: Seq(self.cursor.value().0 + 1),
            });
        }
        Ok(())
    }

    fn flush_contiguous_from_buffer(&mut self, _fx: &mut EffectSink) -> Result<(), ImError> {
        // TODO: 从 buffer 中取连续 seq 并 apply
        Ok(())
    }
}
