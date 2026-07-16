//! ImState — ImModule 的全量纯数据状态；不持 I/O 句柄。

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use crate::send::upload_props::{
    CompletedMedia, FailedMediaOp, PendingMediaOp, PendingMediaPrepare, PendingMediaPut,
    PendingUpload, UploadTarget,
};
use std::collections::{HashMap, HashSet};

mod values;
pub use values::{
    ChannelId, ConnState, Cursor, Id26, InflightSync, SendStatus, Seq, ServerId, TemporaryId,
};

// ─── CorrelationContext ───────────────────────────────────────────────────────
// 外提 sibling `correlation.rs`（守结构闸 baseline + 给 reclaim 新增变体留余量；
// drift-review baseline 棘轮后门收敛）。re-export → `crate::state::CorrelationContext` 调用点零改。
#[path = "correlation.rs"]
mod correlation;
pub use correlation::CorrelationContext;

// ─── ImState ────────────────────────────────────────────────────────────────

/// ImModule 的全量状态（纯数据结构，不含任何 I/O 句柄）
///
/// 所有字段为可序列化的 Rust 值类型，确保三栖确定性。
pub struct ImState {
    /// WS 连接状态机
    pub conn: ConnState,

    /// 服务端 hello 帧回注的 connectionId（A4 握手存入；A3b HTTP 身份头；None = 未握手）。
    pub connection_id: Option<String>,

    /// per-channel 聚合（消息 + 游标 + gate）
    pub channels: HashMap<ChannelId, Channel>,

    /// 当前登录用户在各频道的服务端角色事实；每次 hello 清空，由增量/成员帧重新建立。
    pub viewer_channel_roles: HashMap<ChannelId, String>,

    /// 待对账的发送（temporaryId → PendingSend）
    /// 来自源码：乐观落库后等 echo 或 sync 对账，15s 超时 → unSend
    pub pending_sends: HashMap<TemporaryId, PendingSend>,

    /// 本地上传回包路由：upload corr → 目标消息/附件节点。
    pub pending_uploads: HashMap<helix_core::Correlation, PendingUpload>,

    /// 媒体 Prepare/PUT 回包路由；corr 单次 O(1) remove 后按阶段分发。
    pub pending_media_ops: HashMap<helix_core::Correlation, PendingMediaOp>,

    /// Initial Prepare operations released only after the optimistic message +
    /// pending_media journal batch is acknowledged.
    pub pending_media_after_optimistic: HashMap<TemporaryId, Vec<PendingMediaPrepare>>,

    /// Durable stage transition corr -> operation that may start only after the
    /// journal update succeeds.
    pub pending_media_stage_persists: HashMap<helix_core::Correlation, PendingMediaOp>,

    /// PUT 成功但尚未执行 complete 的媒体；B2 以 uploadId O(1) 继续状态机。
    pub waiting_media_complete: HashMap<String, PendingMediaPut>,

    /// 已通过 complete 严格校验的可信媒体引用；B2b 据此做多附件 barrier/readback。
    pub completed_media: HashMap<String, CompletedMedia>,

    /// 最近失败的媒体阶段；im_retry_send 按消息/target 精确恢复，不越级重放。
    pub failed_media_ops: HashMap<(TemporaryId, UploadTarget), FailedMediaOp>,

    /// Startup `pending_media` scan correlation. Kept outside the generic
    /// correlation map because this is an internal recovery journal route.
    pub pending_media_rehydrate_corr: Option<helix_core::Correlation>,

    /// 媒体 retry 在途消息；重复 im_retry_send 以 temporaryId O(1) 幂等 no-op。
    pub media_retry_inflight: HashSet<TemporaryId>,

    /// 统一 corr 回报路由表：corr → 一条在途异步操作的上下文（`CorrelationContext`）。取代旧
    /// 4 张分散 HashMap + 1 个 `Option`（`scan_corr`）。`PortReply` 到达时单次 `remove(corr)` 取出
    /// 按变体 `match` 分发；一 corr 恰一变体，互斥由一 key 一 value + corr 全局唯一（见枚举不变量）保证。
    pub corr_map: HashMap<helix_core::Correlation, CorrelationContext>,

    /// 同一回复根的最新请求 revision；旧 HTTP 回报不得覆盖新投影。
    pub reply_projection_revisions: HashMap<String, u64>,

    /// 已投影回复 id 集；append 在 Helix 内跨页去重，Angular 只绑定结果。
    pub reply_projection_seen_ids: HashMap<String, HashSet<String>>,

    /// 退避 timer ID（当前重连倒计时）
    pub backoff_timer: Option<helix_core::TimerId>,

    /// ping timer ID
    pub ping_timer: Option<helix_core::TimerId>,

    /// B-rest：本次 increment 批次已收到推送的 channel 集合（`increment_channel` 帧落地）。
    ///
    /// 设计文档 §3 Phase 4：`increment_channel_end`(global) 收尾时，对
    /// `(increment_fetched − need_sync_skip)` 每个 channel 发 proactive sync Http。
    /// 会话边界 reset（hello 起点 clear，§7 跨会话残留）：旧批次残留集会让
    /// 重连后对不该 sync 的 channel 误触发 sync 风暴。
    pub increment_fetched: HashSet<ChannelId>,

    /// B-rest：本次 increment 批次中 `needSync==false`（服务端已追平）的 channel 集合。
    ///
    /// 这些 channel 无需 pull sync——global-end 时从 `increment_fetched` 中扣除。
    /// 与 `increment_fetched` 同生命周期（hello 起点 clear）。
    pub need_sync_skip: HashSet<ChannelId>,

    /// B2 冷启动自愈：每 increment_channel 帧的服务端水位（单调 max），识别「cursor 落后水位」
    /// 补发全量（真源 `heal_incomplete_backfill_once` post.rs:666-715）。hello 起点 clear。
    pub increment_target: HashMap<ChannelId, Seq>,

    /// B2 once-only heal 守卫（sentinel `__im_backfill_heal_v1__`）：防 phantom-heavy channel
    /// 每次 global-end 循环补发；hello 起点 reset 回 false（新会话可再 heal 一次）。
    pub backfill_healed: bool,

    /// 出站 WS 消息单调序号（client→server）。现网 Go `websocket_router.go:34 if r.Seq <= 0`
    /// 拒绝 seq≤0（"Invalid sequence"）→ 关连接。每条出站 WS 帧（ping 等）必带 `seq = ++client_seq`。
    pub client_seq: u64,

    /// B4 全局 sync 并发窗口（VecDeque + 在途上限 K）：解 257 channel 重连风暴。
    /// 与 hello 会话同生命周期（`reset_increment_batch` 同步 reset）。详见 `sync_scheduler.rs`。
    pub sync_scheduler: crate::sync_scheduler::SyncScheduler,

    /// UC-10：本 hello 会话累积 mention+urgent post id，global increment-end 拉 queryTodoList 后清空。
    pub about_me_post_ids: Vec<String>,

    /// hello 早于 cursor scan 回包时，首个 `/channels/load/increment` 只能带空 cursors。
    /// scan 回包加载持久 cursor 后需补发一次 bootstrap increment，避免冷启动只走 sync/notify
    /// 而缺失 `increment_channel` / `im:channel:increment`。
    pub pending_increment_bootstrap_after_scan: bool,

    /// 当前 hello increment 批次尚未提交的 channel/member 写，global end 时合并为一个
    /// `Persist{corr}`；driver ack 前不得发布 increment/loaded 最终投影。
    pub(crate) pending_increment_ops: Vec<helix_core::effect::StorageOp>,

    /// 与 `pending_increment_ops` 同批提交后才允许发布的逐频道投影原始数据。
    pub(crate) pending_increment_projections: Vec<(ChannelId, Vec<u8>)>,

    /// UC-4.5 单频道 hydration 已完成 channel/member 落库、正在等待 sync 终态的 channel。
    /// terminal sync commit/no_change 后移除并触发本地 dialog/messages 读回投影。
    pub hydration_pending: HashSet<ChannelId>,

    /// 当前物理会话中已成功缓存的最近消息窗口；不替代同步 cursor。
    pub(crate) recent_message_coverage:
        HashMap<ChannelId, crate::query::local_first::RecentMessageCoverage>,

    /// coverage 会话代号；hello/disconnect 递增，旧回包不得污染新会话。
    pub(crate) query_session_epoch: u64,
}

impl ImState {
    /// 分配下一个出站 WS 消息序号（从 1 开始单调递增）。
    pub fn next_client_seq(&mut self) -> u64 {
        self.client_seq += 1;
        self.client_seq
    }

    pub fn new() -> Self {
        Self {
            conn: ConnState::Disconnected,
            connection_id: None,
            channels: HashMap::new(),
            viewer_channel_roles: HashMap::new(),
            pending_sends: HashMap::new(),
            pending_uploads: HashMap::new(),
            pending_media_ops: HashMap::new(),
            pending_media_after_optimistic: HashMap::new(),
            pending_media_stage_persists: HashMap::new(),
            waiting_media_complete: HashMap::new(),
            completed_media: HashMap::new(),
            failed_media_ops: HashMap::new(),
            pending_media_rehydrate_corr: None,
            media_retry_inflight: HashSet::new(),
            corr_map: HashMap::new(),
            reply_projection_revisions: HashMap::new(),
            reply_projection_seen_ids: HashMap::new(),
            backoff_timer: None,
            ping_timer: None,
            increment_fetched: HashSet::new(),
            need_sync_skip: HashSet::new(),
            increment_target: HashMap::new(),
            backfill_healed: false,
            client_seq: 0,
            sync_scheduler: crate::sync_scheduler::SyncScheduler::new(),
            about_me_post_ids: Vec::new(),
            pending_increment_bootstrap_after_scan: false,
            pending_increment_ops: Vec::new(),
            pending_increment_projections: Vec::new(),
            hydration_pending: HashSet::new(),
            recent_message_coverage: HashMap::new(),
            query_session_epoch: 0,
        }
    }

    /// 会话边界 reset：每个 hello 窗口起点清空 increment/sync/todo 批次状态。
    pub fn reset_increment_batch(&mut self) {
        self.increment_fetched.clear();
        self.need_sync_skip.clear();
        self.increment_target.clear();
        self.backfill_healed = false;
        self.sync_scheduler.reset();
        self.about_me_post_ids.clear();
        self.pending_increment_bootstrap_after_scan = false;
        self.pending_increment_ops.clear();
        self.pending_increment_projections.clear();
        self.hydration_pending.clear();
        self.viewer_channel_roles.clear();
        self.reply_projection_revisions.clear();
        self.reply_projection_seen_ids.clear();
        self.reset_recent_query_coverage();
    }

    pub(crate) fn reset_recent_query_coverage(&mut self) {
        self.recent_message_coverage.clear();
        self.query_session_epoch = self.query_session_epoch.wrapping_add(1);
        self.corr_map.retain(|_, context| {
            !matches!(
                context,
                CorrelationContext::MessageQueryLocal { .. }
                    | CorrelationContext::MessageQueryRemote { .. }
                    | CorrelationContext::MessageQueryCache { .. }
            )
        });
    }

    pub(crate) fn invalidate_recent_message_coverage(&mut self, channel_id: ChannelId) {
        self.recent_message_coverage.remove(&channel_id);
    }
}

impl Default for ImState {
    fn default() -> Self {
        Self::new()
    }
}

// Id26 单测 + fixture 外提 sibling；re-export 保持旧测试调用点不变。
#[cfg(test)]
#[path = "state_tests.rs"]
mod id26_tests;
#[cfg(test)]
pub(crate) use id26_tests::{test_channel_id, test_server_id};
