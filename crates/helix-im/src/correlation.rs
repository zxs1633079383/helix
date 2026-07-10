//! CorrelationContext —— 单一 corr 路由表的 value（从 `state.rs` 外提，守结构闸 baseline +
//! 给 reclaim 新增变体留余量；drift-review baseline 棘轮后门收敛）。
//! `state.rs` 经 `#[path] mod` 引入 + `pub use` re-export → `crate::state::CorrelationContext`
//! 调用点零改动。

use super::{ChannelId, TemporaryId};

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
        /// E3 续拉：该批 sync 响应 `needs_continuation=true` 时为 true；
        /// on_persist_ok 推进 cursor 后据此决定是否 `maybe_continue_sync`。
        wants_continuation: bool,
        /// sync events 中需要在 channel 写后读回累计行再 emit 的 DialogPatch。
        dialog_patches: Vec<crate::dialog_patch::PendingDialogPatch>,
    },

    /// 在线 post/gate 路径：channel GuardedBump + Get(channel) 的 PortReply 回来后 emit
    /// `im:channel:update-by-post`。该变体承载“写后累计终值”链路，避免前端按事件 delta 自增。
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
    /// sync/notify **HTTP** 回报：清 `inflight_sync` 守卫 + `handle_sync_reply` 解析（取代旧 `corr_to_sync`）。
    SyncPull { channel_id: ChannelId },

    /// P6 `im_query_messages_by_channel` Get 回报 → emit `im:messages:query_result` 投影。
    MessageQuery { channel_id: ChannelId },

    /// UC-3 `im_query_dialog_list` Scan 回报 → emit `im:channels:projection` 完整 dialogList。
    DialogListQuery,

    /// spec06 缺陷A：读族 outbound **HTTP** 回报（request-response 回灌）。
    ///
    /// 读族无 WS 回声、HTTP 响应体即数据 → 注册本上下文，`handle_port_reply` 透传 emit
    /// `im:read:result{req_id, body}`。写族不注册（fire-and-forget）。
    /// `req_id` 前端 bridge 注入（owned String，跨 step 存活，HX-C002；与 corr 解耦）。
    OutboundReadReply { req_id: String },

    /// UC-4.5 陌生 channel 单频道 hydration 的首段 HTTP 回报。
    ///
    /// 仍保留 `im:read:result` 通用回灌；额外把 `data=*IncrementChannel` 送入既有 increment
    /// parser/application 规则，并把 channel/member 写提升为有回报的 Persist，避免未落稳即拉 sync。
    OutboundIncrementHydration { req_id: String },

    /// UC-4.5 IncrementChannel 的 channel/channel_member 相关 Persist 回报。
    /// 成功后才启动单频道 sync；`need_sync=false` 则直接读回本地投影。
    IncrementHydrationPersist {
        channel_id: ChannelId,
        need_sync: bool,
    },

    /// S7（issue #56）：byIds 成员快照读族 **HTTP** 回报。回报时**双 emit**——
    /// ① `im:read:result{req_id, body}`（UC-6.4 ② 冻结契约面照旧透传）；
    /// ② 解析 body 逐 channel emit render-ready `im:channel:members`（壳退纯绑定·载族自愈）。
    /// 与 `OutboundReadReply` 分变体仅为携带「这是 byIds 成员快照」语义（generic 读族不解析 body）。
    OutboundMembersByIds { req_id: String },

    /// S8 回复族（UC-2.4·issue #57）：`im_get_replies` / `im_get_reply_branch` HTTP 回报。
    ///
    /// 读族无 WS 回声、HTTP 响应体即数据 → 注册本上下文，`handle_port_reply` **双 emit**：
    /// `im:read:result{req_id, body}`（冻结 verbatim ② 契约面照旧）+ `im:channel:replies{reqId,
    /// replyIds}`（render-ready 渲染通道·壳纯绑定）。`req_id` 前端 bridge 注入（与 corr 解耦·HX-C002）。
    OutboundReplies { req_id: String },

    /// G-11 定位消息读回：HTTP `posts/get` 返回后由 Helix 整形成 highlighted 消息投影。
    OutboundLocatePost { req_id: String },

    /// UC-10 `posts/queryTodoList` HTTP 回报 → emit `im:todo:updated{items}`（内核自驱、无 req_id）。
    TodoQuery,

    /// D3 `im_load_older_context` 多轮 HTTP 续接（alloc 新 corr / emit older_loaded；HX-C001/C002）。
    LoadOlderContext {
        state: Box<crate::older_context::LoadOlderState>,
    },
}
