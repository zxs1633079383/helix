//! CorrelationContext —— 单一 corr 路由表的 value（从 `state.rs` 外提，守结构闸 baseline +
//! 给 reclaim 新增变体留余量；drift-review baseline 棘轮后门收敛）。
//! `state.rs` 经 `#[path] mod` 引入 + `pub use` re-export → `crate::state::CorrelationContext`
//! 调用点零改动。

use super::{ChannelId, TemporaryId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakeTopicProjection {
    pub req_id: String,
    pub root_message_id: String,
    pub topic_channel_id: ChannelId,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSettingsProjection {
    pub channel_id: ChannelId,
    pub display_name: Option<String>,
    pub purpose: String,
    pub header: String,
    pub setting_version: u64,
    pub channel_capabilities: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncTrigger {
    #[default]
    Routine,
    PongGap,
}

impl SyncTrigger {
    pub(crate) fn projection_label(self) -> Option<&'static str> {
        match self {
            Self::Routine => None,
            Self::PongGap => Some("pong_gap"),
        }
    }
}

/// 单一 corr 路由表的 value——一条在途异步操作（Persist / Http / Scan）回报时
/// 需要的全部上下文。tagged union 取代原 4 张 HashMap + 1 个 Option：
/// 一个 corr 恰属于一个类别，类别互斥由类型系统保证（不再靠 if-let 试探顺序）。
///
/// ## 载体不变量（INVARIANT，承重前提，勿破）
///
/// **所有写入 `corr_map` 的 `Correlation` 均出自同一单调分配器**（`ImModule::alloc_corr` /
/// `alloc_corr_internal` 共用 `self.next_corr`，从 1 单调递增）→ 跨类别 corr 全局唯一永不碰撞。
/// 单表合并（4 张 HashMap + scan `Option` 并一表）的正确性完全依赖此：若任一路径改用独立计数器或
/// host 注入可复用 corr 值，则不同类别 corr 可能撞同一 key → 误路由。新增 corr 来源前必须确认仍由
/// 该分配器产出。HX-C005：变体字段全 owned/Copy（无 `&'a`，要进 HashMap 跨 step 存活，HX-C002）。
// 不 derive `Eq`：`LoadOlderContext` 内含 `serde_json::Value`（f64 非 `Eq`）；本枚举仅作 corr_map value
// （非 key），`PartialEq` 供 ws/registry 测试 `assert_eq!`，`Eq` 无用处（grep 实证）。
#[derive(Debug, Clone, PartialEq)]
pub enum CorrelationContext {
    /// on_start `Scan(_helix_monotonic)` 回报：载入 per-channel cursor + proactive resync（取代旧 `scan_corr`）。
    ScanCursors,

    /// channel sync **Persist** 回报：`channel.on_persist_ok(corr)` 推进 cursor。
    /// （取代旧 `corr_to_channel` + `continuation_pending` 两张表——续拉意图降为本变体的
    /// `wants_continuation` 标志，因为它本就与同一 `persist_corr` 绑定，不是独立路由类别。）
    ChannelPersist {
        channel_id: ChannelId,
        trigger: SyncTrigger,
        /// E3 续拉：该批 sync 响应 `needs_continuation=true` 时为 true；
        /// on_persist_ok 推进 cursor 后据此决定是否 `maybe_continue_sync`。
        wants_continuation: bool,
        /// sync events 中需要在 channel 写后读回累计行再 emit 的 DialogPatch。
        dialog_patches: Vec<crate::dialog_patch::PendingDialogPatch>,
        /// sync HTTP 解析出的 mutation 投影；事务 ack 前不得发布。
        pending_domain_events: Vec<Vec<u8>>,
    },

    /// type7 closed tombstone 的 `PersistAtomic` 回执。成功前不改变内存 cursor/terminal 状态，
    /// 也不发布 renderer Frame；失败则 context 被一次性消费且无后续副作用。
    ChannelTerminalPersist {
        channel_id: ChannelId,
        terminal_seq: crate::state::Seq,
        trigger: SyncTrigger,
        /// 同一权威批次中 terminal 前的可见消息投影；与 tombstone 共用原子回执。
        pending_domain_events: Vec<Vec<u8>>,
    },

    /// Opaque SurfaceResyncRequest 对本地 channel/member/topic 事实的异步读取。
    /// renderer 只提交 surface+scope，不能选择表、SQL 或领域 command。
    SurfaceStorageSnapshot {
        request: crate::surface_runtime::SurfaceResyncRequest,
    },

    /// thread 由既有 getReplies HTTP 权威回包建立 typed slot。
    SurfaceThreadSnapshot {
        request: crate::surface_runtime::SurfaceResyncRequest,
        expected_revision: Option<u64>,
    },

    /// participant receipt 由既有 post/read/list HTTP 权威回包建立 typed slot。
    SurfaceReceiptSnapshot {
        request: crate::surface_runtime::SurfaceResyncRequest,
    },

    /// hello increment 全批 channel/member/cursor 事务回报。
    IncrementBatchPersist {
        projections: Vec<(ChannelId, Vec<u8>)>,
    },

    /// 在线 post/gate 路径：channel GuardedBump + Get(channel) 的 PortReply 回来后发布
    /// `message.left-dialog-list` 属性 frame。该变体承载“写后累计终值”链路，避免前端按事件 delta 自增。
    DialogPatchByPost {
        pending: crate::dialog_patch::PendingDialogPatch,
    },

    /// too_long 后 Helix 主动发起 getLatestPost 首屏重拉的 HTTP 回报。
    TooLongReload {
        channel_id: ChannelId,
        reset_to: crate::state::Seq,
    },

    /// too_long 首屏重拉落库 + channel 写后读回。
    TooLongReloadPersist {
        channel_id: ChannelId,
        dialog_patches: Vec<crate::dialog_patch::PendingDialogPatch>,
    },

    /// P1 乐观落库 **Persist** 回报：把对应 `PendingSend` 状态推进到 `Sending`（取代旧 `corr_to_send`）。
    OptimisticSend { temporary_id: TemporaryId },
    /// F1 `posts/create` **HTTP** 回报：HTTP 200 兜底对账（真 Go self-omit echo → 永 spinner；逻辑见 `send_reconcile`）。
    OutboundSendHttp {
        temporary_id: TemporaryId,
        channel_id: ChannelId,
    },
    /// 发送事实已由本地存储确认后，重新读取已 attach 的 latest timeline。
    ///
    /// 这把 P1、HTTP 对账、WS echo 和失败终态统一收敛为同一条
    /// "durable fact -> local-first projector -> typed Frame" 路径；renderer 不消费
    /// `im:post:*` 补业务状态。`causation_id` 只来自调用方 request id，缺失时保持 None。
    TimelineRefreshAfterSendPersist {
        channel_id: ChannelId,
        causation_id: Option<String>,
    },
    /// G-03a Native 重启后的失败消息重试：先按 temporary_id 查 durable message，
    /// 未命中再按 id 回退。读回后重建 PendingSend 与 posts/create body，复用既有发送链。
    RetrySendRehydrate {
        message_id: String,
        requested_at_ms: u64,
        request_id: Option<String>,
        lookup_by_id: bool,
    },
    /// sync/notify **HTTP** 回报：清 `inflight_sync` 守卫 + `handle_sync_reply` 解析（取代旧 `corr_to_sync`）。
    SyncPull {
        channel_id: ChannelId,
        trigger: SyncTrigger,
    },

    /// 最近消息 local-first 第 1 段：本地 Scan 回报后判定显式窗口 coverage。
    MessageQueryLocal {
        request: Box<crate::query::MessageQueryRequest>,
        query_session_epoch: u64,
        query_generation: u64,
        /// Explicit renderer queries may prove a missing window via HTTP. A
        /// refresh following a confirmed local write must stay local-only,
        /// otherwise one send is amplified into repeated getLatestPost calls.
        allow_remote_fallback: bool,
        /// Command-owned causation carried only for a projector refresh that
        /// follows a confirmed durable mutation. Ordinary renderer queries use None.
        causation_id: Option<String>,
        /// P1 has made an ordinary text send durable. The first authoritative
        /// timeline Frame must leave the core before its posts/create HTTP starts.
        /// It is an internal continuation, never renderer input.
        deferred_send_http: Option<TemporaryId>,
    },

    /// 最近消息 local-first 第 2 段：`getLatestPost` HTTP 回报后合并并按能力缓存。
    MessageQueryRemote {
        request: Box<crate::query::MessageQueryRequest>,
        local_rows_desc: Box<Vec<serde_json::Value>>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<TemporaryId>,
    },

    /// 最近消息 local-first 第 3 段：cache Persist 回报后才记录当前会话 coverage 并 emit。
    MessageQueryCache {
        request: Box<crate::query::MessageQueryRequest>,
        merged_rows_desc: Box<Vec<serde_json::Value>>,
        coverage: Option<crate::query::local_first::RecentMessageCoverage>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<TemporaryId>,
    },

    /// G-05 ID 转发的本地消息逐条 Get 回报；全部组装完成后才发 createPosts HTTP。
    ForwardSourceLookup {
        assembly: Box<crate::forward::ForwardAssembly>,
    },

    /// G08 HTTP 只确认 transport 结果；成功等待 WS authority，失败撤销 request causation。
    OutboundScheduleCreate {
        channel_id: ChannelId,
        request_id: Option<String>,
    },

    /// G08 schedule fact 与 channel read model 原子提交后才允许重读并发布 Frame。
    ScheduleCreatedPersist {
        channel_id: ChannelId,
        revision: u64,
        causation_id: Option<String>,
    },

    /// UC-3 `im_query_dialog_list` Scan 回报 → emit `message.left-dialog-list` 完整 snapshot frame。
    DialogListQuery { causation_id: Option<String> },

    /// spec06 缺陷A：读族 outbound **HTTP** 回报（request-response 回灌）。
    ///
    /// 读族无 WS 回声、HTTP 响应体即数据 → 注册本上下文，`handle_port_reply` 透传 emit
    /// `im:read:result{req_id, body}`。写族不注册（fire-and-forget）。
    /// `req_id` 前端 bridge 注入（owned String，跨 step 存活，HX-C002；与 corr 解耦）。
    OutboundReadReply { req_id: String },

    /// 自己退群写族 HTTP 回报。成功后本地标记 channel.is_remove=1 并发 render-ready left 投影；
    /// 剩余成员的 member delete 仍由服务端 channel_member_update WS 权威驱动。
    OutboundLeave { channel_id: ChannelId },

    /// 建群写族 HTTP 回报。服务端可能在 WebSocket 断线期间成功创建 channel，此时不会有
    /// `channel_created` 实时回声；保留已规范化成员，让 HTTP 权威回包也能落当前 channel/member
    /// 并发同一 render-ready `im:channel:created`，避免前端退回全量 dialog scan。
    OutboundChannelCreate { members: Vec<serde_json::Value> },

    /// G-15 makeTopic HTTP 回包：仅在服务端确认 root/topic 关联后进入本地持久屏障。
    OutboundMakeTopic {
        root_message_id: String,
        req_id: String,
    },

    /// G-15 topic channel + root message 关联持久成功后释放 render-ready replies 投影。
    OutboundMakeTopicPersist { projection: MakeTopicProjection },

    /// 群设置/公告写回：服务端 SUCCESS 必须携带完整权威设置快照与 settingVersion。
    OutboundChannelSettings { channel_id: ChannelId },

    /// 权威设置写入本地 channel 后才释放 settings-updated。
    OutboundChannelSettingsPersist {
        projection: ChannelSettingsProjection,
    },

    /// UC-4.5 陌生 channel 单频道 hydration 的首段 HTTP 回报。
    ///
    /// 仍保留 `im:read:result` 通用回灌；额外把 `data=*IncrementChannel` 送入既有 increment
    /// parser/application 规则，并把 channel/member 写提升为有回报的 Persist，避免未落稳即拉 sync。
    OutboundIncrementHydration { req_id: String },

    /// UC-4.5 IncrementChannel 的 channel/channel_member 相关 Persist 回报。
    /// `need_sync` 已合并服务端提示与本地 cursor 落后事实；成功后才启动单频道 sync，
    /// 两侧均确认无需拉取时才直接读回本地投影。
    IncrementHydrationPersist {
        channel_id: ChannelId,
        need_sync: bool,
        raw_increment: Vec<u8>,
    },

    /// S7（issue #56）：byIds 成员快照读族 **HTTP** 回报。回报时**双 emit**——
    /// ① `im:read:result{req_id, body}`（UC-6.4 ② 冻结契约面照旧透传）；
    /// ② 解析 body 逐 channel emit render-ready `im:channel:members`（壳退纯绑定·载族自愈）。
    /// 与 `OutboundReadReply` 分变体仅为携带「这是 byIds 成员快照」语义（generic 读族不解析 body）。
    OutboundMembersByIds { req_id: String },

    /// G-15a fresh-account 建群候选目录 HTTP 回报 → render-ready 用户投影。
    OutboundContactCandidates { req_id: String },

    /// S8 回复族（UC-2.4·issue #57）：`im_get_replies` / `im_get_reply_branch` HTTP 回报。
    ///
    /// 读族无 WS 回声、HTTP 响应体即数据 → 注册本上下文，`handle_port_reply` **双 emit**：
    /// `im:read:result{req_id, body}`（冻结 verbatim ② 契约面照旧）+ 显式 snapshot/append、
    /// revision、分页元数据的 `im:channel:replies`。请求上下文来自 command+payload，不猜 reqId。
    OutboundReplies {
        request: crate::render_ready_replies::ReplyProjectionRequest,
    },

    /// G-11 定位消息读回：HTTP `posts/get` 返回后由 Helix 整形成 highlighted 消息投影。
    OutboundLocatePost { req_id: String, post_id: String },

    /// G-05 多目标转发 HTTP 接收结果；accepted 不是 delivered，最终送达继续等逐目标 WS post。
    OutboundCreatePosts { req_id: String },

    /// G-14 post/read/list 双 emit：通用 read result + render-ready readerIds/readers。
    OutboundPostReaders { req_id: String },

    /// SurfaceRuntime `viewReceipt` action 的同一 canonical `post/read/list` 回包。它保留 legacy
    /// read/readers emits，并额外投影带 requestId causation 的 typed participant-receipts Frame；HTTP
    /// accepted 本身不允许确认 renderer pending state。
    InteractionViewReceipt {
        request_id: String,
        channel_id: String,
        message_id: String,
    },

    /// Mutation HTTP replies are transport acknowledgements only. The authoritative WS/sync handler owns
    /// persistence and the terminal InteractionFrame, so this corr route intentionally performs no UI work.
    InteractionMutationAck {
        pending: crate::projector::action::PendingAuthorityAction,
    },

    /// `posts_update` carries one channel event sequence and N message mutations. The entire write-set,
    /// including the cursor, is committed atomically; renderer events are released only after this corr succeeds.
    PostsUpdateAtomic {
        channel_id: ChannelId,
        target_seq: crate::state::Seq,
        pending_domain_events: Vec<Vec<u8>>,
        pending_action: Option<crate::projector::action::PendingAuthorityAction>,
    },

    /// Single `post_update` fact and its channel cursor share one durable commit. The render-ready update is
    /// retained until the matching transaction reply succeeds.
    PostUpdateAtomic {
        channel_id: ChannelId,
        target_seq: crate::state::Seq,
        pending_domain_event: Vec<u8>,
        pending_action: Option<crate::projector::action::PendingAuthorityAction>,
    },

    /// UC-10 `posts/queryTodoList` HTTP 回报 → emit `im:todo:updated{items}`（内核自驱、无 req_id）。
    TodoQuery,

    /// D3 `im_load_older_context` 多轮 HTTP 续接（alloc 新 corr / emit older_loaded；HX-C001/C002）。
    LoadOlderContext {
        state: Box<crate::older_context::LoadOlderState>,
    },
}
