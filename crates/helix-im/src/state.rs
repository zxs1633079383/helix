//! ImState — ImModule 的全量纯数据状态；不持 I/O 句柄。

use crate::channel::Channel;
use crate::pending_send::{PendingSend, TimelineReadbackContext};
use crate::send::upload_props::{
    FailedMediaOp, MultipartBatch, MultipartDrain, PendingMediaCompletionCommit, PendingMediaOp,
    PendingMediaPrepare, PendingMediaPut, PendingMediaRetryReset, PendingUpload, UploadTarget,
};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

mod values;
pub use values::{
    is_canonical_post_id, ChannelId, ConnState, Cursor, Id26, InflightSync, SendStatus, Seq,
    ServerId, TemporaryId,
};

// ─── CorrelationContext ───────────────────────────────────────────────────────
// 外提 sibling `correlation.rs`（守结构闸 baseline + 给 reclaim 新增变体留余量；
// drift-review baseline 棘轮后门收敛）。re-export → `crate::state::CorrelationContext` 调用点零改。
#[path = "correlation.rs"]
mod correlation;
pub use correlation::{
    ChannelLifecycleTransition, ChannelSettingsProjection, CorrelationContext,
    HydrationPersistSnapshot, MakeTopicProjection, PendingSendReconciliation, SyncTrigger,
};

/// 同一消息的媒体 props 写入屏障；回包按到达顺序，进度按目标合并。
#[derive(Default)]
pub(crate) struct MediaWriteGate {
    pub owner: Option<helix_core::Correlation>,
    pub replies: VecDeque<(helix_core::Correlation, helix_core::tick::PortOutcome, u64)>,
    pub reply_keys: HashSet<helix_core::Correlation>,
    pub progress: VecDeque<helix_core::Correlation>,
    pub progress_keys: HashSet<helix_core::Correlation>,
}

pub(crate) struct FileUploadProgressState {
    pub target: UploadTarget,
    pub temporary_id: TemporaryId,
    pub channel_id: ChannelId,
    pub part_number: Option<u32>,
    /// PUT 启动时冻结的 readback 上下文；多附件并发时不受后续 retry attempt 覆盖。
    pub timeline_readback: TimelineReadbackContext,
    pub highest_observed_percent: u8,
    pub committed_percent: u8,
    pub inflight_persist: Option<(helix_core::Correlation, u8)>,
    pub queued_latest: Option<u8>,
}

pub(crate) struct FileUploadProgressPersist {
    pub props_after: Option<serde_json::Value>,
    pub upload_corr: helix_core::Correlation,
    pub percent: u8,
}

// ─── ImState ────────────────────────────────────────────────────────────────

/// ImModule 的全量状态（纯数据结构，不含任何 I/O 句柄）
///
/// 所有字段为可序列化的 Rust 值类型，确保三栖确定性。
pub struct ImState {
    pub recent_history: crate::recent_history::HistoryState,
    /// WS 连接状态机
    pub conn: ConnState,

    /// 服务端 hello 帧回注的 connectionId（A4 握手存入；A3b HTTP 身份头；None = 未握手）。
    pub connection_id: Option<String>,

    /// per-channel 聚合（消息 + 游标 + gate）
    pub channels: HashMap<ChannelId, Channel>,

    /// 待对账的发送（temporaryId → PendingSend）
    /// 来自源码：乐观落库后等 echo 或 sync 对账，15s 超时 → unSend
    pub pending_sends: HashMap<TemporaryId, PendingSend>,

    /// 本地上传回包路由：upload corr → 目标消息/附件节点。
    pub pending_uploads: HashMap<helix_core::Correlation, PendingUpload>,

    /// 媒体 Prepare/PUT/Complete 回包路由；corr 单次 O(1) remove 后按阶段分发。
    pub(crate) pending_schedule_media:
        HashMap<helix_core::Correlation, crate::schedule_media::PendingScheduleMedia>,
    /// 定时 multipart 的独立路由；不借用普通 message 的 pending_media/temporary_id 状态。
    pub(crate) pending_schedule_multipart:
        HashMap<helix_core::Correlation, crate::schedule_media::PendingScheduleMultipart>,
    /// 定时 multipart 持有完整 payload 与运行时批次；每个 channel 最多一个 intent。
    pub(crate) schedule_multipart_contexts:
        HashMap<String, crate::schedule_media::ScheduleMultipartContext>,
    /// 稳定媒体引用已落库、createSchedule 已发出的定时 intent；无幂等 wire 时禁止重发。
    pub(crate) schedule_media_submitting: BTreeMap<ChannelId, String>,
    /// Multipart schedule drain retry timers; the timer never carries a signed URL.
    pub(crate) schedule_multipart_retry_timers:
        HashMap<helix_core::TimerId, String>,
    /// Startup scan correlation for durable schedule intents.
    pub(crate) schedule_media_rehydrate_corr: Option<helix_core::Correlation>,
    pub pending_media_ops: HashMap<helix_core::Correlation, PendingMediaOp>,

    /// 同消息的进度/完成/失败写串行化，避免不同存储 worker 交错覆盖 props。
    pub(crate) media_write_gates: HashMap<TemporaryId, MediaWriteGate>,
    pub(crate) media_write_owners: HashMap<helix_core::Correlation, TemporaryId>,

    /// 媒体 PUT 的 5% 进度合并器；每目标最多保留一个最新待提交值。
    pub(crate) file_upload_progress: HashMap<helix_core::Correlation, FileUploadProgressState>,

    /// 文件 PUT 已成功、但仍有进度 Persist 未回执时暂存终态推进。
    ///
    /// message-only 的 5..95% 与 message+pending_media 的 100% 走不同 host worker；
    /// 必须等前者真实回执后再吐终态 Persist，避免慢盘下旧百分比反向覆盖 100。
    pub(crate) media_put_after_progress: HashMap<helix_core::Correlation, PendingMediaPut>,

    /// 进度落库 corr → upload corr；只在 PersistOk 后刷新已 attach timeline。
    pub(crate) file_upload_progress_persists:
        HashMap<helix_core::Correlation, FileUploadProgressPersist>,

    /// Initial Prepare operations released only after the optimistic message +
    /// pending_media journal batch is acknowledged.
    pub pending_media_after_optimistic: HashMap<TemporaryId, Vec<PendingMediaPrepare>>,

    /// Durable stage transition corr -> operation that may start only after the
    /// journal update succeeds.
    pub pending_media_stage_persists: HashMap<helix_core::Correlation, PendingMediaOp>,

    /// Multipart network/timeout retry budget. The timer payload retains only
    /// trusted local input; signed URLs are never carried across a retry.
    pub(crate) multipart_retry_attempts: HashMap<(TemporaryId, UploadTarget), u8>,
    pub(crate) multipart_retry_timers: HashMap<helix_core::TimerId, PendingMediaPrepare>,

    /// One runtime batch per clientUploadId. The signed queue is memory-only;
    /// each ETag leaves this map only after its own PersistAtomic succeeds.
    pub(crate) multipart_batches: HashMap<String, MultipartBatch>,

    /// Failed batches retain their sibling UploadFile routes until every old
    /// range request has replied; only then may retry or UnSend proceed.
    pub(crate) multipart_draining: HashMap<String, MultipartDrain>,

    /// 同一消息最多一条本地完成持久化链在途；prepare/PUT 仍可并发。
    pub media_completion_inflight: HashSet<TemporaryId>,

    /// 其它已进入 Complete 阶段的附件按到达顺序等待，避免并发争写整段 props。
    pub queued_media_completions: HashMap<TemporaryId, VecDeque<PendingMediaPut>>,

    /// 已由 OSS PUT 2xx 确认并完成本地提交的 file id；据此做多附件 barrier/readback。
    pub completed_media: HashSet<String>,

    /// PUT 成功后的 message props + pending_media delete 原子提交回包路由。
    pub pending_media_completion_persists:
        HashMap<helix_core::Correlation, PendingMediaCompletionCommit>,

    /// 用户 retry 先原子提交 Sending/Uploading，再允许 Java/OSS I/O。
    pub pending_media_retry_resets: HashMap<helix_core::Correlation, PendingMediaRetryReset>,

    /// 最近失败的媒体阶段；PUT 失败归一为 Prepare，Complete 失败才原阶段恢复。
    pub failed_media_ops: HashMap<(TemporaryId, UploadTarget), FailedMediaOp>,

    /// Startup `pending_media` scan correlation. Kept outside the generic
    /// correlation map because this is an internal recovery journal route.
    pub pending_media_rehydrate_corr: Option<helix_core::Correlation>,

    /// 启动扫描出的孤儿媒体 journal，必须先把对应 message 原子补偿为 unsend，
    /// PersistOk 后才允许进入 failed_media_ops 并开放 retry。
    pub pending_media_recovery_compensation: Option<(helix_core::Correlation, Vec<FailedMediaOp>)>,

    /// 只有 pending_media 全量 scan 成功且每行都可解码后才为 true。
    /// false 时重启 retry 不得把未完成媒体 props 当成普通消息直发 Go。
    pub media_recovery_ready: bool,

    /// 媒体 retry 在途消息；重复 im_retry_send 以 temporaryId O(1) 幂等 no-op。
    pub media_retry_inflight: HashSet<TemporaryId>,

    /// 统一 corr 回报路由表：corr → 一条在途异步操作的上下文（`CorrelationContext`）。取代旧
    /// 4 张分散 HashMap + 1 个 `Option`（`scan_corr`）。`PortReply` 到达时单次 `remove(corr)` 取出
    /// 按变体 `match` 分发；一 corr 恰一变体，互斥由一 key 一 value + corr 全局唯一（见枚举不变量）保证。
    pub corr_map: HashMap<helix_core::Correlation, CorrelationContext>,

    /// 文字接龙 mutation 的内存状态；终态只在 matching PersistOk 后更新。
    pub(crate) category_chain: crate::category_chain::State,
    pub(crate) chain_mutations: HashMap<String, crate::chain::ChainMutation>,

    /// 已通过 durable projection 的 chain eventId；重复 WS authority 不再重放。
    pub(crate) seen_chain_event_ids: HashSet<String>,
    /// 接龙完整消息投影与专用 chain 投影独立去重，避免双路到达顺序影响消息补全。
    pub(crate) seen_chain_post_event_ids: HashSet<String>,
    pub(crate) pending_chain_post_event_ids: HashSet<String>,

    /// 已进入 PersistAtomic 但尚未收到 PersistOk 的 chain eventId；拦截 HTTP/WS 竞态重复写入。
    pub(crate) pending_chain_event_ids: HashSet<String>,

    /// 每条链已提交的 authority revision；旧回包在进入 PersistAtomic 前丢弃。
    pub(crate) chain_revisions: HashMap<String, i64>,

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
    /// 避免每个 channel commit 都单独发布 channel 列表事件。
    pub(crate) pong_gap_batch: crate::sync::pong_batch::PongGapBatch,

    /// UC-10：本 hello 会话累积 mention+urgent post id，global increment-end 拉 queryTodoList 后清空。
    pub about_me_post_ids: Vec<String>,

    /// hello 早于 cursor scan 回包时，首个 `/channels/load/increment` 只能带空 cursors。
    /// scan 回包加载持久 cursor 后需补发一次 bootstrap increment，避免冷启动只走 sync/notify
    /// 而缺失 `increment_channel` / `im:channel:increment`。
    pub pending_increment_bootstrap_after_scan: bool,
    /// hello 显式协商的恢复分页能力，旧 Go 保持原 WS 协议。
    pub(crate) increment_page_supported: bool,
    pub(crate) increment_pull: Option<crate::increment_pull::IncrementPull>,

    /// 启动 channel 投影扫描成功后才允许向 Go/心跳发送 active cursor 集合。
    /// 扫描失败保持 false，避免未知的删除频道重新进入补偿链路。
    pub startup_channel_projection_ready: bool,

    /// 当前 hello increment 批次尚未提交的 channel/member 写，global end 时合并为一个
    /// `Persist{corr}`；driver ack 前不得发布 increment/loaded 最终投影。
    pub(crate) pending_increment_ops: Vec<helix_core::effect::StorageOp>,

    /// 与 `pending_increment_ops` 同批提交后才允许发布的逐频道投影原始数据。
    pub(crate) pending_increment_projections: Vec<(ChannelId, Vec<u8>)>,

    /// 话题增量批次的活动身份；与普通频道 ready 分开，避免 scoped end 晋升全局列表。
    pub(crate) subtopic_sync_active: Option<String>,

    /// 已完成的话题批次键，重复 end 只读丢弃，不重复发布 ready。
    pub(crate) subtopic_sync_completed: std::collections::HashSet<String>,

    /// 当前缓冲 increment 帧携带的内部 batchId；收尾帧不匹配时 fail-closed。
    pub(crate) pending_increment_batch_id: Option<String>,

    /// 最近一次 global increment 原子落库后建立的频道分页会话；身份/transport 边界会清空。
    pub(crate) channel_sync_session: Option<crate::channel_sync::ChannelSyncSession>,

    /// 频道分页会话代际；每次 ready 递增，旧 cursor 跨代 fail-closed。
    pub(crate) channel_sync_generation: u64,

    /// 当前 increment 批次是否仍有未封口的频道帧；用于拒绝重复 global end。
    pub(crate) channel_sync_batch_pending: bool,

    /// 已封口但尚未收到 Persist 回执的 increment 批次数；每个成功回执各自发布一次 ready。
    pub(crate) channel_sync_persist_inflight: usize,

    /// 当前频道分页尚未 complete 时到达的新批次；complete 后合并为一次新的 ready。
    pub(crate) channel_sync_refresh_pending: bool,

    /// UC-4.5 单频道 hydration 已完成 channel/member 落库、正在等待 sync 终态的 channel。
    /// terminal sync commit/no_change 后移除并触发本地 dialog/messages 读回投影。
    pub hydration_pending: HashSet<ChannelId>,

    /// G13b hydration request identity，贯穿 sync 与 durable read-back 的唯一终态。
    pub hydration_req_ids: HashMap<ChannelId, String>,

    /// 同频道已落库的并发请求，共享主请求的同步及读回终态。
    pub hydration_waiters: HashMap<ChannelId, Vec<String>>,

    /// G09d canonical increment is released only after the same durable read-back as the Result.
    pub hydration_emit_channel_increment: HashSet<ChannelId>,

    /// G09d HTTP authority 的有序成员全集；跨 Persist/sync 保留，durable scan 仅用于集合校验。
    pub hydration_ordered_rosters: HashMap<ChannelId, Vec<serde_json::Value>>,

    /// G09d HTTP authority 的当前 viewer 未读绝对值；仅保留到同轮 durable read-back 对账完成。
    pub hydration_authority_unreads: HashMap<ChannelId, i64>,

    /// 当前物理会话中已成功缓存的最近消息窗口；不替代同步 cursor。
    pub(crate) recent_message_coverage:
        HashMap<ChannelId, crate::query::local_first::RecentMessageCoverage>,

    /// 每个频道已观测到的最新服务端消息时间（毫秒）。
    ///
    /// 本地乐观行的 `create_at` 来自客户端时钟，服务端行来自服务端时钟；客户端时钟落后时，
    /// 刚发出的消息会排到更早的服务端消息（如入群 NOTICE）之前，并可能被
    /// `ORDER BY create_at DESC LIMIT` 的 latest 窗口挤出。发送前用它把本地时间抬到服务端
    /// 时间轴上（单调取最大，O(1)）。
    pub(crate) observed_channel_create_at: HashMap<ChannelId, i64>,

    /// 同一 `(channel, windowToken)` local-first 查询的最新代际。每次发起查询都推进该值，
    /// 使同会话内晚到的旧 Scan/HTTP/cache 回包不能覆盖新窗口结果。
    pub(crate) message_query_generations: HashMap<(ChannelId, String), u64>,

    /// 已附着的有界时间线窗口；只保存分页与定位所需的业务事实。
    pub(crate) timeline_state: crate::timeline_state::TimelineState,

    /// 仅保存已通过远端权限与 durable read-back 的 Timeline coverage；不保存视觉 window。
    pub(crate) timeline_navigation_coverage:
        HashMap<String, crate::timeline_navigation::TimelineNavigationCoverage>,

    /// Timeline V3 reqId ledger；同一 reqId 只允许一条 accepted→terminal 链。
    pub(crate) timeline_navigation_pending: HashSet<String>,

    /// G05 request-scoped, bounded target ledger. Only trusted WS authority may consume an entry;
    /// HTTP acceptance merely prunes explicit failures and never establishes delivery.
    pub(crate) pending_forward_deliveries: crate::forward::PendingForwardDeliveryLedger,

    /// G08 command request 只为下一条匹配 authority 事件提供 causation，不建立成功事实。
    pub(crate) pending_schedule_requests: BTreeMap<ChannelId, String>,

    /// G09 cancel request 与 G08 create request 分槽，避免竞态时串用 causation。
    pub(crate) pending_schedule_cancel_requests: BTreeMap<ChannelId, String>,

    /// 已提交与在途 schedule revision 分开保存，保证失败可重放且重复 WS 不产生双写。
    pub(crate) committed_schedule_revisions: BTreeMap<ChannelId, u64>,
    pub(crate) inflight_schedule_revisions: BTreeMap<ChannelId, u64>,

    /// G-15a 创建权威的提交账本；区分在途与已提交以允许失败重放并抑制 HTTP/WS 竞态。
    pub(crate) inflight_channel_creates: HashSet<ChannelId>,
    pub(crate) committed_channel_creates: HashSet<ChannelId>,

    /// G16b 已提交/在途 WS event 序号分账，失败可重放且重复 authority 不重复出帧。
    pub(crate) inflight_member_update_seqs: HashSet<(ChannelId, Seq)>,
    pub(crate) committed_member_update_seqs: HashSet<(ChannelId, Seq)>,

    /// 同一频道在线角色变更只允许一条成员表读写链在途；重复 authority 等首链完成后由后续
    /// increment/member snapshot 收敛，避免并发读改写覆盖新角色。
    pub(crate) inflight_member_role_updates: HashSet<ChannelId>,

    /// 已经通过持久屏障的 G14 authority 版本；用于 O(1) 丢弃重复或倒退的 WS 回执。
    pub(crate) committed_post_reads: BTreeMap<String, (i64, String)>,

    /// 当前账号/租户下每频道已投影的公告绝对版本；只接受单调递增快照。
    pub(crate) announcement_versions: BTreeMap<ChannelId, u64>,

    /// 已发起但尚未收到绝对列表的公告重拉版本；抑制重复/旧 changed 通知。
    pub(crate) announcement_reload_versions: BTreeMap<ChannelId, u64>,

    /// 每频道置顶集合失效代次；阻止 pin/unpin 后迟到的旧 HTTP 列表重新落库。
    pub(crate) pinned_projection_epochs: BTreeMap<ChannelId, u64>,

    /// coverage 会话代号；hello/disconnect 递增，旧回包不得污染新会话。
    pub(crate) query_session_epoch: u64,
}

impl ImState {
    /// 失效一个频道的置顶绝对集合，并推进代次使旧 HTTP 回包不能重新落库。
    pub(crate) fn invalidate_pinned_projection(
        &mut self,
        account_id: &str,
        channel_id: ChannelId,
    ) -> Option<helix_core::Effect> {
        let epoch = self.pinned_projection_epochs.entry(channel_id).or_default();
        *epoch = epoch.wrapping_add(1);
        crate::query::pinned_projection::projection_key(account_id, channel_id)
            .ok()
            .map(|key| crate::query::pinned_projection::invalidate_effect(account_id, key))
    }

    /// 分配下一个出站 WS 消息序号（从 1 开始单调递增）。
    pub fn next_client_seq(&mut self) -> u64 {
        self.client_seq += 1;
        self.client_seq
    }

    /// 创建不含平台句柄的空 IM 状态，所有 authority 辅助账本从确定性空值开始。
    pub fn new() -> Self {
        Self {
            conn: ConnState::Disconnected,
            connection_id: None,
            channels: HashMap::new(),
            pending_sends: HashMap::new(),
            pending_uploads: HashMap::new(),
            pending_schedule_media: HashMap::new(),
            pending_schedule_multipart: HashMap::new(),
            schedule_multipart_contexts: HashMap::new(),
            schedule_media_submitting: BTreeMap::new(),
            schedule_multipart_retry_timers: HashMap::new(),
            schedule_media_rehydrate_corr: None,
            pending_media_ops: HashMap::new(),
            media_write_gates: HashMap::new(),
            media_write_owners: HashMap::new(),
            file_upload_progress: HashMap::new(),
            media_put_after_progress: HashMap::new(),
            file_upload_progress_persists: HashMap::new(),
            pending_media_after_optimistic: HashMap::new(),
            pending_media_stage_persists: HashMap::new(),
            multipart_retry_attempts: HashMap::new(),
            multipart_retry_timers: HashMap::new(),
            multipart_batches: HashMap::new(),
            multipart_draining: HashMap::new(),
            media_completion_inflight: HashSet::new(),
            queued_media_completions: HashMap::new(),
            completed_media: HashSet::new(),
            pending_media_completion_persists: HashMap::new(),
            pending_media_retry_resets: HashMap::new(),
            failed_media_ops: HashMap::new(),
            pending_media_rehydrate_corr: None,
            pending_media_recovery_compensation: None,
            media_recovery_ready: false,
            media_retry_inflight: HashSet::new(),
            recent_history: Default::default(),
            corr_map: HashMap::new(),
            category_chain: crate::category_chain::State::default(),
            chain_mutations: HashMap::new(),
            seen_chain_event_ids: HashSet::new(),
            seen_chain_post_event_ids: HashSet::new(),
            pending_chain_post_event_ids: HashSet::new(),
            pending_chain_event_ids: HashSet::new(),
            chain_revisions: HashMap::new(),
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
            increment_page_supported: false,
            increment_pull: None,
            startup_channel_projection_ready: false,
            pending_increment_ops: Vec::new(),
            pending_increment_projections: Vec::new(),
            subtopic_sync_active: None,
            subtopic_sync_completed: HashSet::new(),
            pending_increment_batch_id: None,
            channel_sync_session: None,
            channel_sync_generation: 0,
            channel_sync_batch_pending: true,
            channel_sync_persist_inflight: 0,
            channel_sync_refresh_pending: false,
            hydration_pending: HashSet::new(),
            hydration_req_ids: HashMap::new(),
            hydration_waiters: HashMap::new(),
            hydration_emit_channel_increment: HashSet::new(),
            hydration_ordered_rosters: HashMap::new(),
            hydration_authority_unreads: HashMap::new(),
            recent_message_coverage: HashMap::new(),
            observed_channel_create_at: HashMap::new(),
            message_query_generations: HashMap::new(),
            timeline_state: crate::timeline_state::TimelineState::default(),
            timeline_navigation_coverage: HashMap::new(),
            timeline_navigation_pending: HashSet::new(),
            pending_forward_deliveries: crate::forward::PendingForwardDeliveryLedger::default(),
            pending_schedule_requests: BTreeMap::new(),
            pending_schedule_cancel_requests: BTreeMap::new(),
            committed_schedule_revisions: BTreeMap::new(),
            inflight_schedule_revisions: BTreeMap::new(),
            inflight_channel_creates: HashSet::new(),
            committed_channel_creates: HashSet::new(),
            inflight_member_update_seqs: HashSet::new(),
            committed_member_update_seqs: HashSet::new(),
            inflight_member_role_updates: HashSet::new(),
            committed_post_reads: BTreeMap::new(),
            announcement_versions: BTreeMap::new(),
            announcement_reload_versions: BTreeMap::new(),
            pinned_projection_epochs: BTreeMap::new(),
            query_session_epoch: 0,
        }
    }

    /// 会话边界 reset：每个 hello 窗口起点清空 increment/sync/todo 批次状态。
    pub fn reset_increment_batch(&mut self) {
        self.increment_pull = None;
        self.corr_map.retain(|_, context| {
            !matches!(
                context,
                CorrelationContext::IncrementPullHttp | CorrelationContext::IncrementPullPersist
            )
        });
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
        self.inflight_member_role_updates.clear();
        self.subtopic_sync_active = None;
        self.subtopic_sync_completed.clear();
        self.pending_increment_batch_id = None;
        self.channel_sync_batch_pending = true;
        self.channel_sync_persist_inflight = 0;
        self.hydration_pending.clear();
        self.hydration_req_ids.clear();
        self.hydration_waiters.clear();
        self.hydration_emit_channel_increment.clear();
        self.hydration_ordered_rosters.clear();
        self.hydration_authority_unreads.clear();
        self.reply_projection_revisions.clear();
        self.reply_projection_seen_ids.clear();
        self.seen_chain_event_ids.clear();
        self.seen_chain_post_event_ids.clear();
        self.pending_chain_post_event_ids.clear();
        self.pending_chain_event_ids.clear();
        self.chain_revisions.clear();
        self.reset_transport_query_session();
    }

    pub(crate) fn reset_recent_query_coverage(&mut self) {
        self.reset_query_session_correlations();
        self.timeline_state.reset();
    }

    /// A transport reconnect invalidates query coverage and in-flight replies, but it is not an
    /// identity boundary. The renderer keeps the same attached slots and revisions while the
    /// socket reconnects, so preserving projector state lets recovered durable facts publish the
    /// next V2 patch against the revision that is still mounted in the UI.
    pub(crate) fn reset_transport_query_session(&mut self) {
        // 增量批次/transport reset 会让普通 timeline query 失效，但本地发送读回不依赖 WS，
        // 仍需保留 deferred posts/create 衔接，否则已落库的发送会丢掉出站入口。
        self.drop_query_session_correlations(true);
        self.announcement_reload_versions.clear();
        self.pinned_projection_epochs.clear();
    }

    /// 完整 query/session reset 同时撤销旧查询回包。
    fn reset_query_session_correlations(&mut self) {
        self.drop_query_session_correlations(false);
        self.clear_pending_action_state();
    }

    /// 撤销旧窗口查询；transport reset 保留本地发送衔接，身份 reset 全部撤销。
    fn drop_query_session_correlations(&mut self, preserve_deferred_send_http: bool) {
        self.recent_message_coverage.clear();
        self.timeline_navigation_coverage.clear();
        self.timeline_navigation_pending.clear();
        self.message_query_generations.clear();
        self.corr_map.retain(|_, context| match context {
            // 本地发送读回不依赖 WS；仅 transport reset 保留，身份切换仍删除。
            CorrelationContext::MessageQueryLocal {
                deferred_send_http, ..
            } if preserve_deferred_send_http => deferred_send_http.is_some(),
            CorrelationContext::MessageQueryLocal { .. }
            | CorrelationContext::MessageQueryRemote { .. }
            | CorrelationContext::MessageQueryCache { .. }
            | CorrelationContext::MessageQueryReadback { .. } => false,
            _ => true,
        });
        self.query_session_epoch = self.query_session_epoch.wrapping_add(1);
    }

    /// 清理依赖旧 transport authority 回包的请求辅助态。
    fn clear_pending_action_state(&mut self) {
        self.pending_forward_deliveries = crate::forward::PendingForwardDeliveryLedger::default();
        self.pending_schedule_requests.clear();
        self.pending_schedule_media.clear();
        self.pending_schedule_multipart.clear();
        self.schedule_multipart_contexts.clear();
        self.schedule_multipart_retry_timers.clear();
        self.schedule_media_submitting.clear();
        self.schedule_media_rehydrate_corr = None;
        self.pending_schedule_cancel_requests.clear();
        self.inflight_schedule_revisions.clear();
        self.inflight_channel_creates.clear();
    }

    /// 身份切换时清除旧 viewer 的附着窗口、相关性与覆盖证明。
    pub(crate) fn reset_message_v3_identity(&mut self) {
        self.increment_pull = None;
        self.multipart_retry_attempts.clear();
        self.multipart_retry_timers.clear();
        self.multipart_batches.clear();
        self.multipart_draining.clear();
        self.pending_schedule_multipart.clear();
        self.schedule_multipart_contexts.clear();
        self.schedule_multipart_retry_timers.clear();
        self.schedule_media_submitting.clear();
        self.schedule_media_rehydrate_corr = None;
        self.recovery_session.invalidate();
        // Tenant/actor facts are never portable.  A B session must rediscover
        // channels and cursors from its own store rather than treating A's
        // in-memory aggregates as a locally trusted snapshot.
        self.channels.clear();
        // 服务端时间下界属于上一身份观测到的频道事实，不能跨账号复用。
        self.observed_channel_create_at.clear();
        self.committed_schedule_revisions.clear();
        self.committed_channel_creates.clear();
        self.inflight_member_update_seqs.clear();
        self.committed_member_update_seqs.clear();
        self.inflight_member_role_updates.clear();
        self.committed_post_reads.clear();
        self.chain_mutations.clear();
        self.seen_chain_event_ids.clear();
        self.seen_chain_post_event_ids.clear();
        self.pending_chain_post_event_ids.clear();
        self.pending_chain_event_ids.clear();
        self.chain_revisions.clear();
        self.announcement_versions.clear();
        self.announcement_reload_versions.clear();
        self.sync_scheduler.reset();
        self.hydration_pending.clear();
        self.hydration_req_ids.clear();
        self.hydration_waiters.clear();
        self.hydration_emit_channel_increment.clear();
        self.hydration_ordered_rosters.clear();
        self.hydration_authority_unreads.clear();
        self.pending_increment_ops.clear();
        self.pending_increment_projections.clear();
        self.subtopic_sync_active = None;
        self.subtopic_sync_completed.clear();
        self.pending_increment_batch_id = None;
        self.channel_sync_session = None;
        self.channel_sync_generation = self.channel_sync_generation.wrapping_add(1);
        self.channel_sync_batch_pending = false;
        self.channel_sync_persist_inflight = 0;
        self.channel_sync_refresh_pending = false;
        self.reset_recent_query_coverage();
        self.corr_map.retain(|_, context| {
            !matches!(
                context,
                CorrelationContext::ChannelPersist { .. }
                    | CorrelationContext::ChainHttp { .. }
                    | CorrelationContext::ChainPersist { .. }
                    | CorrelationContext::ChainMutationPersist { .. }
                    | CorrelationContext::ChannelTerminalPersist { .. }
                    | CorrelationContext::MessageV3RevokeChannelPersist { .. }
                    | CorrelationContext::MessageV3RevokeChannelReadback
                    | CorrelationContext::MessageV3SyncDialogReadback
                    | CorrelationContext::SyncPull { .. }
                    | CorrelationContext::ScanChannelProjections
                    | CorrelationContext::IncrementMessageTimestampScan { .. }
                    | CorrelationContext::IncrementPullHttp
                    | CorrelationContext::IncrementPullPersist
                    | CorrelationContext::TooLongReload { .. }
                    | CorrelationContext::TooLongReloadPersist { .. }
                    | CorrelationContext::ChannelUpdateByPost { .. }
                    | CorrelationContext::TopicActivityRootRead { .. }
                    | CorrelationContext::TopicActivityPersist { .. }
                    | CorrelationContext::IncrementBatchPersist { .. }
                    | CorrelationContext::ChannelSyncPage { .. }
                    | CorrelationContext::ChannelSyncPageMemberSnapshot { .. }
                    | CorrelationContext::IncrementHydrationPersist { .. }
                    | CorrelationContext::HydrationChannelReadback { .. }
                    | CorrelationContext::HydrationMemberReadback { .. }
                    | CorrelationContext::HydrationMessagesReadback { .. }
                    | CorrelationContext::HydrationCursorReadback { .. }
                    | CorrelationContext::OutboundIncrementHydration { .. }
                    | CorrelationContext::UpdateChannelDialogPersist { .. }
                    | CorrelationContext::MemberProjectionPersist { .. }
                    | CorrelationContext::MemberProjectionReadback { .. }
                    | CorrelationContext::CanonicalStreamPersist { .. }
                    | CorrelationContext::CanonicalChainPostHydration { .. }
                    | CorrelationContext::NotifyChannelPersist { .. }
                    | CorrelationContext::DialogListQuery { .. }
                    | CorrelationContext::SubtopicsQuery { .. }
                    | CorrelationContext::PostReadPersist { .. }
                    | CorrelationContext::OutboundExactPosts { .. }
                    | CorrelationContext::ExactPostsPersist { .. }
                    | CorrelationContext::ExactPostsReadback { .. }
                    | CorrelationContext::OutboundChannelCreate { .. }
                    | CorrelationContext::ChannelCreatePersist { .. }
                    | CorrelationContext::ChannelMemberUpdatePersist { .. }
                    | CorrelationContext::ChannelMemberRoleScan { .. }
                    | CorrelationContext::ChannelMemberRolePersist { .. }
                    | CorrelationContext::ChannelMemberRoleChannelReadback { .. }
                    | CorrelationContext::ChannelMemberRoleReadback { .. }
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

    /// 清除指定频道的分页/定位绝对窗口，防止消息突变后复用旧 durable row 快照。
    pub(crate) fn invalidate_timeline_navigation_coverage(&mut self, channel_id: ChannelId) {
        self.timeline_navigation_coverage
            .retain(|_, coverage| coverage.channel_id != channel_id);
    }

    /// 登记服务端权威消息时间，单调取最大；非正值（未定序/缺省）忽略。
    ///
    /// 每次观测 O(1) 写；只在服务端行或权威广播上调用，不读 DOM、不发查询。
    pub(crate) fn observe_channel_create_at(&mut self, channel_id: ChannelId, create_at: i64) {
        if create_at <= 0 {
            return;
        }
        let observed = self
            .observed_channel_create_at
            .entry(channel_id)
            .or_insert(create_at);
        if create_at > *observed {
            *observed = create_at;
        }
    }

    /// 返回该频道本地乐观消息必须不早于的服务端时间下界（毫秒）。
    ///
    /// 返回 `None` 表示本会话尚未观测到该频道的服务端消息（此时只能用本地时钟）。
    pub(crate) fn channel_create_at_floor(&self, channel_id: ChannelId) -> Option<i64> {
        self.observed_channel_create_at
            .get(&channel_id)
            .copied()
            .filter(|create_at| *create_at > 0)
    }

    /// 丢弃全部频道的服务端时间下界；只在会话/身份/频道集合整体失效时调用。
    pub(crate) fn reset_observed_channel_create_at(&mut self) {
        self.observed_channel_create_at.clear();
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
