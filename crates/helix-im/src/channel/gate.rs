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

use crate::state::{ChannelId, Cursor, InflightSync, Seq};
use crate::sync_session::EventEnvelope;
use helix_core::effect::TimerId;
use helix_core::Correlation;
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
    /// 已由 type7 原子提交确认的 closed tombstone 水位。
    ///
    /// `Some` 后本地不再接收任何普通事件或发起 resync，避免晚到 post/sync 把已关闭的
    /// channel 当作活跃频道重新投影。持久化镜像位于 `channel_event_cursor.terminal_event_seq`；
    /// 该列只记录 closed 这个唯一合法终态，不存服务端任意 payload。
    pub terminal_event_seq: Option<Seq>,
}

mod commit;
mod flow;
mod projection;
mod storage;
pub use projection::emit_for_kind;
pub use storage::{
    apply_read_op, edit_content_op, event_to_storage_op, event_to_upsert_op, pin_state_op,
    revoke_op,
};

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
