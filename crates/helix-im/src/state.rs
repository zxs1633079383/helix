//! ImState — ImModule 的全量纯数据状态；不持 I/O 句柄。

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use std::collections::{HashMap, HashSet};

// ─── 值对象（全部 newtype，类型层禁止混淆）────────────────────────────────

/// mattermost 26 字符 `[a-z0-9]` ID；Copy newtype，热路径零分配 key（HX-C005）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id26([u8; 26]);

impl Id26 {
    /// 边界零信任构造：仅恰 26 字节且全 `[a-z0-9]` 时 Some。
    pub fn from_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 26
            || !b
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        {
            return None;
        }
        let mut arr = [0u8; 26];
        arr.copy_from_slice(b);
        Some(Self(arr))
    }

    /// 零拷贝取字符串视图。
    pub fn as_str(&self) -> &str {
        // SAFETY: 不变量——Id26 只能经 from_str（已验证恰 26 字节 [a-z0-9]）构造，
        // 故内部恒为合法 UTF-8。
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }

    pub fn as_bytes(&self) -> &[u8; 26] {
        &self.0
    }

    /// 派生确定性 u64（FNV-1a，非加密）——用于 timer id 偏移等需数值的场景。
    pub fn hash_u64(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in &self.0 {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
}

/// 频道 ID（mattermost 26 字符 ID，不透明 Copy）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(pub Id26);

impl ChannelId {
    /// 从字符串构造（边界零信任，非法→None）。
    pub fn from_str(s: &str) -> Option<Self> {
        Id26::from_str(s).map(Self)
    }
    /// 零拷贝取字符串视图。
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    /// 零拷贝取 26 字节定长数组（allHash FNV 流式喂入等场景，零分配）。
    pub fn as_bytes(&self) -> &[u8; 26] {
        self.0.as_bytes()
    }
    /// 派生确定性 u64（timer id 偏移等数值场景）。
    pub fn hash_u64(&self) -> u64 {
        self.0.hash_u64()
    }
}

/// Server 消息 ID（echo 到时的真实 ID）；真 Go msgId 是 26 字符串，不能用旧 u64。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerId(pub Id26);

impl ServerId {
    /// 从字符串构造（边界零信任，非法→None）。
    pub fn from_str(s: &str) -> Option<Self> {
        Id26::from_str(s).map(Self)
    }
    /// 零拷贝取字符串视图。
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

/// UI 注入的临时消息 ID（乐观落库 PK，不由内核生成）
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemporaryId(pub String);

/// per-channel 事件序号（单调递增）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Seq(pub u64);

impl Seq {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// 消息发送状态；helix-im v1 仅覆盖文字消息，文件上传状态机由宿主处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    Local,
    Sending,
    Sent,
    UnSend,
}

/// per-channel sync 在途标记（B1 gate_inflight 守卫）
///
/// 同一 channel 同时只允许一个在途 sync Http（防止并发自踩）。
/// 值：当前在途 sync 的 corr（用于日志）；None = 无在途 sync。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflightSync(pub helix_core::Correlation);

/// per-channel 游标（单调推进，MAX guard）
#[derive(Debug, Clone, Copy)]
pub struct Cursor {
    pub value: Seq,
}

impl Cursor {
    pub fn new(value: Seq) -> Self {
        Self { value }
    }

    /// MAX guard：仅在 next > current 时推进
    pub fn try_advance(&mut self, next: Seq) -> bool {
        if next > self.value {
            self.value = next;
            true
        } else {
            false
        }
    }

    pub fn value(&self) -> Seq {
        self.value
    }
}

/// WS 连接状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Disconnected,
    Connecting,
    Connected,
    Closing,
}

// ─── CorrelationContext ───────────────────────────────────────────────────────

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
    },

    /// P1 乐观落库 **Persist** 回报：把对应 `PendingSend` 状态推进到 `Sending`（取代旧 `corr_to_send`）。
    OptimisticSend { temporary_id: TemporaryId },
    /// F1 `posts/create` **HTTP** 回报：HTTP 200 兜底对账（真 Go self-omit echo → 永 spinner；逻辑见 `send_reconcile`）。
    OutboundSendHttp { temporary_id: TemporaryId },
    /// sync/notify **HTTP** 回报：清 `inflight_sync` 守卫 + `handle_sync_reply` 解析（取代旧 `corr_to_sync`）。
    SyncPull { channel_id: ChannelId },

    /// P6 投影查询 **Get** 回报：`im_query_messages_by_channel` 的 `Get(message WHERE channel_id)`
    /// 回报 → 把行 emit 成 `im:messages:query_result` 投影（data 数组 → driver `{items:[]}` 包装）。
    MessageQuery { channel_id: ChannelId },

    /// UC-3 投影查询 **Scan** 回报：`im_query_dialog_list` 的 `Scan(channel ORDER BY last_post_at)`
    /// 回报 → 把整份 channel 行 emit 成 `im:channels:projection`（`data.dialogList` 完整 DialogItem
    /// 数组，含 unread_count 等，前端零计算）。无 channel_id 字段（全量会话列表，非按群）。
    DialogListQuery,

    /// spec06 缺陷A：读族 outbound **HTTP** 回报（request-response 回灌）。
    ///
    /// 读命令（top20/getReplies/bookmark/announcement/user-misc 读/bot 读…）**无 WS 回声**，
    /// HTTP **响应体本身就是数据**——故读族（且仅读族）alloc corr 时注册本上下文，
    /// `handle_port_reply` 据此把响应体透传 emit 成 `im:read:result{req_id, body}`（前端 bridge 等
    /// 该 req_id resolve 后直接渲染）。写命令不注册（fire-and-forget，数据走 WS 回声）。
    ///
    /// `req_id` 是前端 bridge 注入的请求标识（owned String，进 corr_map 跨 step 存活，HX-C002）；
    /// 与 corr 解耦（前端不见 core 内部 corr）。透传不在 helix 改业务数据（后端权威，HX-C005）。
    OutboundReadReply { req_id: String },

    /// UC-10 通知/待办 **HTTP** 回报：global increment-end 触发的 `posts/queryTodoList` 回报 →
    /// 装配 emit `im:todo:updated{items}`（`todo::emit_todo_updated`）。独立变体（装配非透传、内核自驱无 req_id）。
    TodoQuery,

    /// D3 上拉翻页（`im_load_older_context`）**HTTP** 回报：多轮 `posts/postContext` 编排续接。
    /// 每轮回报 → `ingest_round` 决定续（alloc 新 corr 发下一轮 + 重注册）/停（emit older_loaded）；
    /// sans-IO「8 轮循环」靠 corr_map 续接非同步 `for`（HX-C001）。`state` owned 跨 PortReply（HX-C002）。
    LoadOlderContext {
        state: Box<crate::older_context::LoadOlderState>,
    },
}

// ─── ImState ────────────────────────────────────────────────────────────────

/// ImModule 的全量状态（纯数据结构，不含任何 I/O 句柄）
///
/// 所有字段为可序列化的 Rust 值类型，确保三栖确定性。
pub struct ImState {
    /// WS 连接状态机
    pub conn: ConnState,

    /// 服务端 hello 帧回注的 connectionId（A4 握手存入）。A3b：作 IM HTTP 身份头盖章进
    /// sync/notify、posts/create。None = 尚未握手；前端凭 established + 此值放行 pendingSends。
    pub connection_id: Option<String>,

    /// per-channel 聚合（消息 + 游标 + gate）
    pub channels: HashMap<ChannelId, Channel>,

    /// 待对账的发送（temporaryId → PendingSend）
    /// 来自源码：乐观落库后等 echo 或 sync 对账，15s 超时 → unSend
    pub pending_sends: HashMap<TemporaryId, PendingSend>,

    /// 统一 corr 回报路由表：corr → 一条在途异步操作的上下文（`CorrelationContext`）。取代旧
    /// 4 张分散 HashMap + 1 个 `Option`（`scan_corr`）。`PortReply` 到达时单次 `remove(corr)` 取出
    /// 按变体 `match` 分发；一 corr 恰一变体，互斥由一 key 一 value + corr 全局唯一（见枚举不变量）保证。
    pub corr_map: HashMap<helix_core::Correlation, CorrelationContext>,

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

    /// B2 冷启动自愈：本批次每 increment_channel 帧带来的服务端权威水位（`inc.last_event_seq`，
    /// 单调 max 累计）。真源 `heal_incomplete_backfill_once`（post.rs:666-715）：冷启动「cursor 已推
    /// maxSeq 但本地只灌首屏」卡死 channel 靠「本地落后水位」识别补发全量；sans-IO 核等价信号 =
    /// 「local cursor < 服务端水位」。与 `increment_fetched` 同生命周期（hello clear），防跨会话误判。
    pub increment_target: HashMap<ChannelId, Seq>,

    /// B2 冷启动自愈：本批次是否已跑过一次 heal（once-only 守卫）。
    ///
    /// 对齐真源 sentinel `__im_backfill_heal_v1__`（post.rs:667/708）：phantom-heavy channel
    /// 本地 cursor 恒 < 水位（HX-C008 §1.6：phantom 不推 cursor），若每次 global-end 都补发
    /// 会进死循环 → 每批次只 heal 一次。hello 起点 reset 回 false（新会话允许再 heal 一次）。
    pub backfill_healed: bool,

    /// 出站 WS 消息单调序号（client→server）。现网 Go `websocket_router.go:34 if r.Seq <= 0`
    /// 拒绝 seq≤0（"Invalid sequence"）→ 关连接。每条出站 WS 帧（ping 等）必带 `seq = ++client_seq`。
    pub client_seq: u64,

    /// B4 全局 sync 并发窗口（VecDeque + 在途上限 K）：解 257 channel 重连风暴。
    /// 与 hello 会话同生命周期（`reset_increment_batch` 同步 reset）。详见 `sync_scheduler.rs`。
    pub sync_scheduler: crate::sync_scheduler::SyncScheduler,

    /// UC-10：本 hello 会话累积 mention+urgent post id，global increment-end 拉 queryTodoList 后清空。
    pub about_me_post_ids: Vec<String>,
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
            pending_sends: HashMap::new(),
            corr_map: HashMap::new(),
            backoff_timer: None,
            ping_timer: None,
            increment_fetched: HashSet::new(),
            need_sync_skip: HashSet::new(),
            increment_target: HashMap::new(),
            backfill_healed: false,
            client_seq: 0,
            sync_scheduler: crate::sync_scheduler::SyncScheduler::new(),
            about_me_post_ids: Vec::new(),
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
