//! ImState — ImModule 的全量纯数据状态；不持 I/O 句柄。

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use crate::send::upload_props::{
    CompletedMedia, FailedMediaOp, PendingMediaOp, PendingMediaPrepare, PendingMediaPut,
    PendingUpload, UploadTarget,
};
use std::collections::{BTreeMap, HashMap, HashSet};

mod values;
pub use values::{
    ChannelId, ConnState, Cursor, Id26, InflightSync, SendStatus, Seq, ServerId, TemporaryId,
};

// ─── CorrelationContext ───────────────────────────────────────────────────────
// 外提 sibling `correlation.rs`（守结构闸 baseline + 给 reclaim 新增变体留余量；
// drift-review baseline 棘轮后门收敛）。re-export → `crate::state::CorrelationContext` 调用点零改。
#[path = "correlation.rs"]
mod correlation;
pub use correlation::{
    ChannelSettingsProjection, CorrelationContext, MakeTopicProjection, SyncTrigger,
};

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

    /// 本批 `increment_channel` 的首次到达顺序。Go 按最近活跃优先交付；此 Vec 保留该顺序，
    /// 让新鲜频道优先进入有界 sync 窗口。与 `increment_fetched` 配对做 O(1) 去重，
    /// 输入帧序本身是确定性回放事实，不依赖 HashSet 迭代序。
    pub increment_order: Vec<ChannelId>,

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

    /// 当前 reconnect/relogin 的 authority recovery 会话。它不保存业务事实，只把比较、
    /// 持久化回执和 renderer V2 epoch 绑定到同一个身份边界。
    pub recovery_session: crate::sync_session::RecoverySession,

    /// `hashMismatch` fan-out 的批次屏障。只跟踪在途 Persist correlation，
    /// 避免每个 channel commit 都单独发布 dialog Frame。
    pub(crate) pong_gap_batch: crate::sync::pong_batch::PongGapBatch,

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

    /// 同一 `(channel, windowToken)` local-first 查询的最新代际。每次发起查询都推进该值，
    /// 使同会话内晚到的旧 Scan/HTTP/cache 回包不能覆盖新窗口结果。
    pub(crate) message_query_generations: HashMap<(ChannelId, String), u64>,

    /// 左侧会话列表已发布的 render-ready 行与 revision。只用于构造原子 frame，不保存 raw storage 行。
    pub(crate) dialog_list_frame: crate::dialog_frame::DialogListFrameState,

    /// 已 attach 的有界时间线窗口；只保存 typed VM/revision，raw 行仍由 query/render-ready 边界消费。
    pub(crate) timeline_projector: crate::projector::timeline::TimelineProjector,

    /// 回复抽屉的 HTTP 权威快照/replaceView slot；只存 typed VM，不保留原始回复 body。
    pub(crate) thread_detail_projector: crate::projector::thread_detail::ThreadDetailProjector,

    /// channel/member/topic 的本地事实重放 slot。它们与 timeline 同属 identity-scoped
    /// SurfaceRuntime，不能跨登录会话复用 revision。
    pub(crate) channel_summary_projector:
        crate::projector::channel_summary::ChannelSummaryProjector,
    pub(crate) composer_projector: crate::projector::composer::ComposerProjector,
    pub(crate) member_roster_projector: crate::projector::member_roster::MemberRosterProjector,
    pub(crate) topic_list_projector: crate::projector::topic_list::TopicListProjector,

    /// Frame 下发时登记的 opaque action lease 与本 session replay ledger。它与 timeline slot 同属
    /// SurfaceRuntime，会话/身份边界必须一起清除。
    pub(crate) action_registry: crate::projector::action::ActionBindingRegistry,

    /// invoke 受理结果的有界 protocol receipt 队列。receipt 不是业务完成，真正终态仍等 causation
    /// Frame；此队列仅保证前端能可靠知道当前请求是否被 Helix 接受。
    pub(crate) action_receipts: crate::projector::action::ActionReceiptBus,

    /// In-flight mutation causation keyed by authority family and message id. Renderer input can never choose
    /// this key; it is derived from the trusted ActionDispatch grant.
    pub(crate) pending_authority_actions: BTreeMap<
        (crate::projector::action::AuthorityMutationKind, String),
        crate::projector::action::PendingAuthorityAction,
    >,

    /// G05 request-scoped, bounded target ledger. Only trusted WS authority may consume an entry;
    /// HTTP acceptance merely prunes explicit failures and never establishes delivery.
    pub(crate) pending_forward_deliveries: crate::forward::PendingForwardDeliveryLedger,

    /// action 的权威 HTTP 回包以 typed participant-receipts Frame 发布，不能把 raw read/list body
    /// 直接交给 renderer 推导。
    pub(crate) participant_receipts_projector:
        crate::projector::participant_receipts::ParticipantReceiptsProjector,

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
            increment_order: Vec::new(),
            need_sync_skip: HashSet::new(),
            increment_target: HashMap::new(),
            backfill_healed: false,
            client_seq: 0,
            sync_scheduler: crate::sync_scheduler::SyncScheduler::new(),
            recovery_session: crate::sync_session::RecoverySession::default(),
            pong_gap_batch: crate::sync::pong_batch::PongGapBatch::default(),
            about_me_post_ids: Vec::new(),
            pending_increment_bootstrap_after_scan: false,
            pending_increment_ops: Vec::new(),
            pending_increment_projections: Vec::new(),
            hydration_pending: HashSet::new(),
            recent_message_coverage: HashMap::new(),
            message_query_generations: HashMap::new(),
            dialog_list_frame: crate::dialog_frame::DialogListFrameState::default(),
            timeline_projector: crate::projector::timeline::TimelineProjector::default(),
            thread_detail_projector:
                crate::projector::thread_detail::ThreadDetailProjector::default(),
            channel_summary_projector:
                crate::projector::channel_summary::ChannelSummaryProjector::default(),
            composer_projector: crate::projector::composer::ComposerProjector::default(),
            member_roster_projector:
                crate::projector::member_roster::MemberRosterProjector::default(),
            topic_list_projector: crate::projector::topic_list::TopicListProjector::default(),
            action_registry: crate::projector::action::ActionBindingRegistry::new(
                crate::projector::action::DEFAULT_ACTION_REGISTRY_CAPACITY,
            )
            .expect("static action registry capacity must be non-zero"),
            action_receipts: crate::projector::action::ActionReceiptBus::new(
                crate::projector::action::DEFAULT_ACTION_RECEIPT_CAPACITY,
            )
            .expect("static action receipt capacity must be non-zero"),
            pending_authority_actions: BTreeMap::new(),
            pending_forward_deliveries: crate::forward::PendingForwardDeliveryLedger::default(),
            participant_receipts_projector:
                crate::projector::participant_receipts::ParticipantReceiptsProjector::default(),
            query_session_epoch: 0,
        }
    }

    /// 会话边界 reset：每个 hello 窗口起点清空 increment/sync/todo 批次状态。
    pub fn reset_increment_batch(&mut self) {
        self.increment_fetched.clear();
        self.increment_order.clear();
        self.need_sync_skip.clear();
        self.increment_target.clear();
        self.backfill_healed = false;
        self.sync_scheduler.reset();
        self.pong_gap_batch.reset();
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
        self.message_query_generations.clear();
        // Old action requestIds may be released with the registry only after their authority corr routes are gone.
        // Otherwise a late HTTP reply could reuse the causation of a newly accepted action in the next session.
        self.corr_map.retain(|_, context| {
            !matches!(
                context,
                CorrelationContext::MessageQueryLocal { .. }
                    | CorrelationContext::MessageQueryRemote { .. }
                    | CorrelationContext::MessageQueryCache { .. }
                    | CorrelationContext::InteractionViewReceipt { .. }
            )
        });
        self.timeline_projector.reset();
        self.thread_detail_projector.reset();
        self.channel_summary_projector.reset();
        self.composer_projector.reset();
        self.member_roster_projector.reset();
        self.topic_list_projector.reset();
        self.action_registry.clear();
        self.action_receipts.clear();
        self.pending_authority_actions.clear();
        self.pending_forward_deliveries = crate::forward::PendingForwardDeliveryLedger::default();
        self.participant_receipts_projector.reset();
        self.query_session_epoch = self.query_session_epoch.wrapping_add(1);
    }

    /// 身份 scope 切换是 SurfaceRuntime 的硬边界：旧 viewer 的 attach slot、读侧相关性与
    /// coverage 都不能在新 viewer 下继续出帧。该清理不影响发送/同步等非 query 业务 corr。
    pub(crate) fn reset_surface_runtime_for_identity_change(&mut self) {
        self.recovery_session.invalidate();
        // Tenant/actor facts are never portable.  A B session must rediscover
        // channels and cursors from its own store rather than treating A's
        // in-memory aggregates as a locally trusted snapshot.
        self.channels.clear();
        self.sync_scheduler.reset();
        self.hydration_pending.clear();
        self.pending_increment_ops.clear();
        self.pending_increment_projections.clear();
        self.reset_recent_query_coverage();
        self.dialog_list_frame.reset();
        self.corr_map.retain(|_, context| {
            !matches!(
                context,
                CorrelationContext::ChannelPersist { .. }
                    | CorrelationContext::ChannelTerminalPersist { .. }
                    | CorrelationContext::SyncPull { .. }
                    | CorrelationContext::IncrementBatchPersist { .. }
                    | CorrelationContext::IncrementHydrationPersist { .. }
                    | CorrelationContext::OutboundIncrementHydration { .. }
                    | CorrelationContext::DialogListQuery { .. }
                    | CorrelationContext::SurfaceStorageSnapshot { .. }
                    | CorrelationContext::SurfaceThreadSnapshot { .. }
                    | CorrelationContext::SurfaceReceiptSnapshot { .. }
            )
        });
    }

    /// 为一个 timeline query slot 分配本会话最新代际。极端代际耗尽时先整体失效旧 query
    /// corr，再从 1 重启，避免回绕值与仍在途的旧回包重新相等。
    pub(crate) fn begin_message_query_generation(
        &mut self,
        channel_id: ChannelId,
        window_token: &str,
    ) -> u64 {
        let key = (channel_id, window_token.to_string());
        let Some(current) = self.message_query_generations.get(&key).copied() else {
            self.message_query_generations.insert(key, 1);
            return 1;
        };
        let Some(next) = current.checked_add(1) else {
            self.reset_recent_query_coverage();
            self.message_query_generations.insert(key, 1);
            return 1;
        };
        self.message_query_generations.insert(key, next);
        next
    }

    pub(crate) fn is_current_message_query_generation(
        &self,
        channel_id: ChannelId,
        window_token: &str,
        generation: u64,
    ) -> bool {
        self.message_query_generations
            .get(&(channel_id, window_token.to_string()))
            .is_some_and(|current| *current == generation)
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
