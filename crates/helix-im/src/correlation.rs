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
    pub display_name: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSettingsProjection {
    pub channel_id: ChannelId,
    pub display_name: Option<String>,
    pub purpose: String,
    pub header: String,
    pub setting_version: u64,
    pub causation_id: Option<String>,
}

/// G16 durable lifecycle transition; close and self-leave remain distinct authority facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelLifecycleTransition {
    Closed { delete_at: i64 },
    Left,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SyncTrigger {
    #[default]
    Routine,
    PongGap,
    /// G13b hydration sync; terminal projection is emitted only after durable read-back.
    Hydration,
}

impl SyncTrigger {
    pub(crate) fn projection_label(self) -> Option<&'static str> {
        match self {
            Self::Routine => None,
            Self::PongGap => Some("pong_gap"),
            Self::Hydration => None,
        }
    }
}

/// Hydration 的 channel/member 预写内存快照；失败时回滚本地临时壳。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HydrationPersistSnapshot {
    pub had_channel: bool,
    pub previous_target: Option<crate::state::Seq>,
    pub was_increment_fetched: bool,
    pub was_need_sync_skip: bool,
    pub previous_about_me_len: usize,
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
    /// MessageV3 业务写提交屏障；只有 matching PersistOk 才按原顺序释放事件。
    MessageV3Commit { terminal_events: Vec<Vec<u8>> },

    /// G-01/G-02 原子提交后的读回路由，并保留一次生成的 post 投影。
    MessageV3PostPersist {
        event: Box<crate::sync_session::EventEnvelope>,
        received_data: Box<serde_json::Value>,
    },

    /// G-01/G-02/G-05 按 viewer 读回绝对态，并在成功后消费可信 forward target。
    MessageV3PostReadback {
        received_data: Box<serde_json::Value>,
        channel_id: ChannelId,
        causation_id: Option<String>,
    },

    /// G-04 update_channel.lastPost 已进入 channel/current-viewer member 原子写集。
    MessageV3RevokeChannelPersist { channel_id: ChannelId },

    /// G-04 复合键读回只释放 channel 绝对态，不重复发布 post revoke。
    MessageV3RevokeChannelReadback,

    /// G-06 quickReply 与 cursor 的原子提交屏障。
    MessageV3ReactionPersist {
        event: Box<crate::sync_session::EventEnvelope>,
    },

    /// G-06 提交成功后的 durable message 单行读回。
    MessageV3ReactionReadback { message_id: String },

    /// G-07 expedite_map 与 cursor 的原子提交屏障。
    MessageV3UrgentPersist {
        event: Box<crate::sync_session::EventEnvelope>,
    },

    /// G-07 提交成功后的 durable message 单行读回。
    MessageV3UrgentReadback { message_id: String },

    /// G-14a 模板 props 与 cursor 的原子提交屏障。
    MessageV3TemplatePersist {
        event: Box<crate::sync_session::EventEnvelope>,
    },

    /// G-14a 提交成功后的 durable message 单行读回。
    MessageV3TemplateReadback,

    /// on_start `Scan(_helix_monotonic)` 回报：载入 per-channel cursor + proactive resync（取代旧 `scan_corr`）。
    ScanCursors,

    /// channel sync **Persist** 回报：推进 cursor，并把下一条连续实时事件交回 MessageV3 写屏障。
    /// （取代旧 `corr_to_channel` + `continuation_pending` 两张表——续拉意图降为本变体的
    /// `wants_continuation` 标志，因为它本就与同一 `persist_corr` 绑定，不是独立路由类别。）
    ChannelPersist {
        channel_id: ChannelId,
        trigger: SyncTrigger,
        /// E3 续拉：该批 sync 响应 `needs_continuation=true` 时为 true；
        /// on_persist_ok 推进 cursor 后据此决定是否 `maybe_continue_sync`。
        wants_continuation: bool,
        /// sync events 中需要在 channel 写后读回累计行再 emit 的 ChannelUpdate。
        channel_updates: Vec<crate::channel_update::PendingChannelUpdate>,
        /// sync HTTP 解析出的 mutation 投影；事务 ack 前不得发布。
        pending_domain_events: Vec<Vec<u8>>,
    },

    /// type7 closed tombstone 的 `PersistAtomic` 回执。成功前不改变内存 cursor/terminal 状态，
    /// 也不发布终态事件；失败则 context 被一次性消费且无后续副作用。
    ChannelTerminalPersist {
        channel_id: ChannelId,
        terminal_seq: crate::state::Seq,
        trigger: SyncTrigger,
        /// 同一权威批次中 terminal 前的可见消息投影；与 tombstone 共用原子回执。
        pending_domain_events: Vec<Vec<u8>>,
    },

    /// hello increment 全批 channel/member/cursor 事务回报。
    IncrementBatchPersist {
        projections: Vec<(ChannelId, Vec<u8>)>,
    },

    /// 在线 post/gate 路径：channel GuardedBump + Get(channel) 的 PortReply 回来后发布
    /// `message.left-dialog-list` 属性 frame。该变体承载“写后累计终值”链路，避免前端按事件 delta 自增。
    ChannelUpdateByPost {
        pending: crate::channel_update::PendingChannelUpdate,
    },

    /// `update_channel` 的公共 channel 稀疏字段与当前 viewer `channel_member`
    /// 绝对态已进入同一相关 Persist；仅成功回执允许发布 channel/read 双事件。
    UpdateChannelDialogPersist {
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member_channel: Option<Box<serde_json::Value>>,
        causation_id: Option<String>,
    },

    /// too_long 后 Helix 主动发起 getLatestPost 首屏重拉的 HTTP 回报。
    TooLongReload {
        channel_id: ChannelId,
        reset_to: crate::state::Seq,
    },

    /// too_long 首屏重拉落库 + channel 写后读回。
    TooLongReloadPersist {
        channel_id: ChannelId,
        /// 权威窗口对应的游标；只在同批 PersistAtomic 成功后应用。
        reset_to: crate::state::Seq,
        channel_updates: Vec<crate::channel_update::PendingChannelUpdate>,
    },

    /// P1 乐观落库 **Persist** 回报：把对应 `PendingSend` 状态推进到 `Sending`（取代旧 `corr_to_send`）。
    OptimisticSend { temporary_id: TemporaryId },
    /// `posts/create` HTTP admission 回报；成功不结算 sent，失败进入本地失败态。
    OutboundSendHttp {
        temporary_id: TemporaryId,
        channel_id: ChannelId,
        /// 仅显式 retry 为 true；admission 成功后强制从 Go 权威历史按 temporaryId 对账。
        authoritative_readback: bool,
    },
    /// 权威历史已命中 retry 消息后的终态持久化回报；timeline 已由同一远端查询发布。
    AuthoritativeSendReconcilePersist { temporary_id: TemporaryId },
    /// 发送事实已由本地存储确认后，重新读取已 attach 的 timeline。
    ///
    /// 这把 P1、HTTP 对账、WS echo 和失败终态统一收敛为同一条
    /// “durable fact -> local-first readback -> MessageV3 event”路径；consumer 不消费
    /// `im:post:*` 补业务状态。`causation_id` 只来自调用方 request id，缺失时保持 None。
    TimelineRefreshAfterSendPersist {
        channel_id: ChannelId,
        /// Action retry 保留授权 binding 的原 window；legacy send/retry 为 None，按 latest 处理。
        window_token: Option<String>,
        causation_id: Option<String>,
    },
    /// 首笔媒体 message+journal 事务失败后的原子补偿 barrier。
    /// 成功前 `media_retry_inflight` 保持占用，禁止 retry 启动任何 Java/OSS I/O；
    /// 失败则继续 fail-closed，防止没有 durable journal 的上传逃逸。
    MediaFailureCompensation {
        temporary_id: TemporaryId,
        channel_id: ChannelId,
        window_token: Option<String>,
        causation_id: Option<String>,
    },
    /// G-03a Native 重启后的失败消息重试：先按 temporary_id 查 durable message，
    /// 未命中再按 id 回退。读回后重建 PendingSend 与 posts/create body，复用既有发送链。
    RetrySendRehydrate {
        message_id: String,
        requested_at_ms: u64,
        request_id: Option<String>,
        /// action 的可信 scope channel；普通 command 没有 action ledger，保持 None。
        action_channel_id: Option<ChannelId>,
        window_token: Option<String>,
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
        /// timeline optimistic event must leave the core before its posts/create HTTP starts.
        /// It is an internal continuation, never renderer input.
        deferred_send_http: Option<TemporaryId>,
    },

    /// 最近消息 local-first 第 2 段：`getLatestPost` HTTP 回报后仅写入 durable message。
    MessageQueryRemote {
        request: Box<crate::query::MessageQueryRequest>,
        local_rows_desc: Box<Vec<serde_json::Value>>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<TemporaryId>,
        /// 显式 retry 的权威对账键；普通 timeline 查询保持 None。
        authoritative_send_readback: Option<TemporaryId>,
    },

    /// 最近消息 local-first 第 3 段：cache Persist 回报后发起同一 Scan read-back。
    MessageQueryCache {
        request: Box<crate::query::MessageQueryRequest>,
        coverage: Option<crate::query::local_first::RecentMessageCoverage>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<TemporaryId>,
    },

    /// 最近消息 local-first 第 4 段：durable Scan 回报才允许发布唯一 timeline 终态。
    MessageQueryReadback {
        request: Box<crate::query::MessageQueryRequest>,
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

    /// G-05 合并转发来源频道 Get 回报；标题闭合后才允许装配 createPosts。
    ForwardSourceChannelLookup {
        assembly: Box<crate::forward::ForwardAssembly>,
    },

    /// G08 HTTP 只确认 transport 结果；成功等待 WS authority，失败撤销 request causation。
    OutboundScheduleCreate {
        channel_id: ChannelId,
        request_id: Option<String>,
    },

    /// G09 HTTP 只确认 transport 结果；成功等待 WS authority，失败撤销 cancel causation。
    OutboundScheduleCancel {
        channel_id: ChannelId,
        request_id: Option<String>,
    },

    /// G08 schedule fact 与 channel read model 原子提交后才允许重读并发布事件。
    ScheduleCreatedPersist {
        channel_id: ChannelId,
        revision: u64,
        causation_id: Option<String>,
    },

    /// G08 原子提交成功后读取 creator 本地 channel_schedule 绝对态。
    MessageV3ScheduleCreatedReadback,

    /// G09 canceled fact 与 channel read model 原子提交后才允许重读并发布事件。
    ScheduleCanceledPersist {
        channel_id: ChannelId,
        revision: u64,
        causation_id: Option<String>,
    },

    /// G09 原子提交成功后读取 creator 本地 canceled 绝对态。
    MessageV3ScheduleCanceledReadback,

    /// UC-3 `im_query_dialog_list` Scan 回报 → emit channel list 与 viewer read 绝对态。
    DialogListQuery { causation_id: Option<String> },

    /// spec06 缺陷A：读族 outbound **HTTP** 回报（request-response 回灌）。
    ///
    /// 读族无 WS 回声、HTTP 响应体即数据 → 注册本上下文，`handle_port_reply` 按命令身份
    /// 透传或投影 `im:read:result{req_id, body}`。写族不注册（fire-and-forget）。
    /// `req_id` 前端 bridge 注入（owned String，跨 step 存活，HX-C002；与 corr 解耦）。
    OutboundReadReply {
        req_id: String,
        command: String,
        channel_id: Option<String>,
    },

    /// MV3-G11g exact-by-id HTTP 回报；locate 标志不改变此独立查询语义。
    OutboundExactPosts {
        req_id: String,
        requested_ids: Vec<String>,
    },

    /// MV3-G11g HTTP 可见命中已写入本地 message，等待按临时主键逐项读回。
    ExactPostsPersist {
        req_id: String,
        accepted_keys: Vec<String>,
    },

    /// MV3-G11g exact read-back continuation；结果只在最后一个 key 读回后结算一次。
    ExactPostsReadback {
        req_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        rows: Vec<serde_json::Value>,
    },

    /// MV3-G11h initial-window HTTP 回报；只知 postId 的结果独立于 locate/exact。
    OutboundInitialWindow {
        req_id: String,
        post_id: String,
        page_size: u32,
    },

    /// MV3-G11h HTTP rows 已写入本地 message，等待有界 durable identity read-back。
    InitialWindowPersist {
        req_id: String,
        post_id: String,
        accepted_keys: Vec<String>,
    },

    /// MV3-G11h initial-window read-back continuation，最终只结算一次 Result。
    InitialWindowReadback {
        req_id: String,
        post_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        rows: Vec<serde_json::Value>,
    },

    /// MV3-G02e 草稿 ScopedGet 读回；命令携带账号、频道和 req_id 的完整隔离上下文。
    MessageV3DraftReadback {
        command: Box<crate::draft::QueryDraftCommand>,
    },

    /// 自己退群写族 HTTP 回报；SUCCESS 只排队本地持久屏障，不直接发布 renderer 状态。
    OutboundLeave { channel_id: ChannelId },

    /// G16 关闭/退出生命周期事实提交后才允许发布 channel 终态事件。
    ChannelLifecyclePersist {
        channel_id: ChannelId,
        transition: ChannelLifecycleTransition,
        causation_id: Option<String>,
    },

    /// G16b 成员变更写集的单次持久屏障；成功前成员终态事件不可见。
    ChannelMemberUpdatePersist {
        channel_id: ChannelId,
        event_seq: Option<crate::state::Seq>,
        raw_channel: Vec<u8>,
        members: Vec<crate::render_ready_members::RenderMember>,
        leaves: Vec<String>,
        causation_id: Option<String>,
    },

    /// 建群写族 HTTP 回报。服务端可能在 WebSocket 断线期间成功创建 channel，此时不会有
    /// `channel_created` 实时回声；保留已规范化成员，让 HTTP 权威回包也能落当前 channel/member
    /// 并发同一 render-ready `im:channel:created`，避免前端退回全量 dialog scan。
    OutboundChannelCreate {
        members: Vec<serde_json::Value>,
        request_id: Option<String>,
    },

    /// G-15a channel/member 权威事实原子提交后才允许发布 MessageV3 绝对态。
    ChannelCreatePersist {
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member_rows: Vec<serde_json::Value>,
        causation_id: Option<String>,
    },

    /// G-15 makeTopic HTTP 回包：仅在服务端确认 root/topic 关联后进入本地持久屏障。
    OutboundMakeTopic {
        root_message_id: String,
        req_id: String,
        display_name: String,
    },

    /// G-15 topic channel + root message 关联持久成功后读取 durable root fact。
    OutboundMakeTopicPersist { projection: MakeTopicProjection },

    /// G-15 durable root readback 只允许释放 topic MessageV3 事件。
    OutboundMakeTopicRootRead { projection: MakeTopicProjection },

    /// 群设置/公告写回：服务端 SUCCESS 必须携带完整权威设置快照与 settingVersion。
    OutboundChannelSettings {
        channel_id: ChannelId,
        causation_id: Option<String>,
    },

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
        req_id: String,
        need_sync: bool,
        raw_increment: Vec<u8>,
        snapshot: HydrationPersistSnapshot,
    },

    /// G13b channel durable read-back stage 1: tenant/viewer-scoped channel row.
    HydrationChannelReadback {
        req_id: String,
        channel_id: ChannelId,
    },

    /// G13b durable read-back stage 2: current viewer member row.
    HydrationMemberReadback {
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
    },

    /// G13b durable read-back stage 3: bounded message window.
    HydrationMessagesReadback {
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member: Box<serde_json::Value>,
    },

    /// G13b durable read-back stage 4: persisted cursor, then one terminal Result.
    HydrationCursorReadback {
        req_id: String,
        channel_id: ChannelId,
        channel: Box<serde_json::Value>,
        member: Box<serde_json::Value>,
        messages: Box<serde_json::Value>,
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
    /// 读族无 WS 回声、HTTP 响应体即 authority；本上下文冻结请求身份、snapshot/append
    /// 模式与 revision，PortReply 只允许生成完整 thread MessageV3 事件。
    OutboundReplies {
        request: crate::render_ready_replies::ReplyProjectionRequest,
    },

    /// G-11 定位消息读回：HTTP `posts/get` 返回后由 Helix 整形成 highlighted 消息投影。
    OutboundLocatePost { req_id: String, post_id: String },

    /// G-05 多目标转发 HTTP 接收结果；accepted 不是 delivered，最终送达继续等逐目标 WS post。
    OutboundCreatePosts { req_id: String },

    /// Topic 子频道 dialog activity 提交后读取 parent root，避免客户端反向拼接 topic relation。
    TopicActivityRootRead {
        projection: crate::topic_activity::TopicActivityProjection,
    },

    /// Parent root card activity 持久成功后刷新父群 timeline 与 topic dialog 计数。
    TopicActivityPersist {
        parent_channel_id: ChannelId,
        parent_topic_message_count: u64,
    },

    /// G-14 post/read/list 双 emit：通用 read result + render-ready readerIds/readers。
    OutboundPostReaders { req_id: String },

    /// G14 `post_read` 权威位图必须先提交到 message.read_bits；成功后才允许发布当前 viewer 事件。
    PostReadPersist {
        channel_id: ChannelId,
        message_id: String,
        receipt_revision: i64,
        read_bits: String,
        terminal_event: Vec<u8>,
    },

    /// `posts_update` carries one channel event sequence and N message mutations. The entire write-set,
    /// including the cursor, is committed atomically; renderer events are released only after this corr succeeds.
    PostsUpdateAtomic {
        channel_id: ChannelId,
        target_seq: crate::state::Seq,
        pending_domain_events: Vec<Vec<u8>>,
        refresh_timeline: bool,
    },

    /// Single `post_update` fact and its channel cursor share one durable commit. The render-ready update is
    /// retained until the matching transaction reply succeeds.
    PostUpdateAtomic {
        channel_id: ChannelId,
        target_seq: crate::state::Seq,
        pending_domain_event: Vec<u8>,
    },

    /// UC-10 `posts/queryTodoList` HTTP 回报 → emit `im:todo:updated{items}`（内核自驱、无 req_id）。
    TodoQuery,

    /// G10a 多轮 HTTP 续接；终点只能进入 message persist barrier，不能直接发布 raw rows。
    LoadOlderContext {
        state: Box<crate::older_context::LoadOlderState>,
    },

    /// G10a 旧消息 write barrier；成功后才允许读取完整 bounded window。
    LoadOlderCache {
        state: Box<crate::older_context::LoadOlderState>,
    },

    /// G10a 持久化后的完整 timeline readback；只从该结果构造 MessageV3 事件。
    LoadOlderReadback {
        state: Box<crate::older_context::LoadOlderState>,
    },

    /// G10b/G11a V3 HTTP 回报；成功页只允许进入 message upsert barrier。
    TimelineNavigationHttp {
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
    },

    /// 导航页 message upsert 的 matching PersistOk 屏障。
    TimelineNavigationCache {
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
    },

    /// 导航页持久化后的单行 readback；成功才允许发布 Timeline Delta。
    TimelineNavigationReadback {
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
    },
}
