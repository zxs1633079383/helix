//! ImState — ImModule 的全量状态（纯数据，无 I/O 句柄）
//!
//! 对应源码实测的核心状态字段：
//! - channels: per-channel cursor + gate + buffer
//! - pending_sends: temporaryId → PendingSend（等待 echo/sync 对账）
//! - conn_state: WS 连接状态机
//! - corr_map: 统一 corr 回报路由表（corr → CorrelationContext）

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use std::collections::{HashMap, HashSet};

// ─── 值对象（全部 newtype，类型层禁止混淆）────────────────────────────────

/// mattermost 风格定长 ID（26 字符 `[a-z0-9]`）。
///
/// Copy newtype，零分配热路径 key（HX-C005）；真 Go channelId/userId/msgId 恒为 26 char。
/// 不透明：构造只经 `from_str`（边界零信任）。ADR-009。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id26([u8; 26]);

impl Id26 {
    /// 边界零信任构造：仅当 `s` 恰 26 字节且全部为 `[a-z0-9]` 时 Some，否则 None（不 panic）。
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
    /// 派生确定性 u64（timer id 偏移等数值场景）。
    pub fn hash_u64(&self) -> u64 {
        self.0.hash_u64()
    }
}

/// Server 消息 ID（echo 到时的真实 ID）——mattermost msgId 是 26 字符 base32 字符串
/// （含 a-z，非数字），同 ChannelId 用 Id26（ADR-009）；旧 u64 装不下真 Go msgId
/// 会让 C1 echo→reconcile 整条死（expert-panel-review 2026-06-16 critical 确认）。
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

#[cfg(test)]
pub(crate) fn test_channel_id(n: u64) -> ChannelId {
    let s = format!("chfixx{n:020x}");
    ChannelId::from_str(&s).expect("test channel id fixture must be valid Id26")
}

#[cfg(test)]
pub(crate) fn test_server_id(n: u64) -> ServerId {
    let s = format!("srvfix{n:020x}");
    ServerId::from_str(&s).expect("test server id fixture must be valid Id26")
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

/// 消息发送状态（4 态 enum）
///
/// ## 边界声明
///
/// helix-im v1 仅覆盖文字消息，不含文件上传状态（uploading）。
/// 文件上传状态机在 helix 范围外，由宿主层（helix-tauri）处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendStatus {
    /// 已乐观落库，未发送
    Local,
    /// 已发出 HTTP POST，等待 echo
    Sending,
    /// echo/sync 对账成功
    Sent,
    /// 15s 超时，标记失败（无自动补发）
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
/// **所有写入 `corr_map` 的 `Correlation` 均出自同一单调分配器**
/// （`ImModule::alloc_corr` / `alloc_corr_internal` 共用同一 `self.next_corr`，从 1 单调递增），
/// 因此跨类别 corr 全局唯一、永不碰撞。单表合并（4 张分散 HashMap + scan 的 `Option` 并入
/// 一张表）的正确性完全依赖这一点：若将来任一注册路径改用独立计数器、或由 host 注入可复用
/// 的 corr 值，则不同类别的 corr 可能撞同一 key → 误路由（scan corr 撞 send corr 等）。
/// 新增任何 corr 来源前，必须确认其仍由该单调分配器产出。
///
/// HX-C005 热路径：变体字段全部 owned/Copy（`ChannelId` Copy / `TemporaryId` 是已分配的
/// String，移交所有权零额外分配），无 `&'a` 生命周期（HX-C002，要进 HashMap 跨 step 存活）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorrelationContext {
    /// on_start `Scan(_helix_monotonic)` 的回报：载入 per-channel cursor + proactive resync。
    /// （取代旧 `scan_corr: Option<Correlation>`——单值语义并入统一表，靠 corr 唯一性区分。）
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

    /// P1 乐观落库 **Persist** 回报：把对应 `PendingSend` 状态推进到 `Sending`。
    /// （取代旧 `corr_to_send`。）
    OptimisticSend { temporary_id: TemporaryId },

    /// sync/notify **HTTP** 回报：清 `inflight_sync` 守卫 + `handle_sync_reply` 解析。
    /// （取代旧 `corr_to_sync`。）
    SyncPull { channel_id: ChannelId },
}

// ─── ImState ────────────────────────────────────────────────────────────────

/// ImModule 的全量状态（纯数据结构，不含任何 I/O 句柄）
///
/// 所有字段为可序列化的 Rust 值类型，确保三栖确定性。
pub struct ImState {
    /// WS 连接状态机
    pub conn: ConnState,

    /// 服务端 hello 帧回注的 connectionId（A4 握手完成后存入）。
    /// A3b：作 IM HTTP 身份头（`connectionId`）盖章进 sync/notify、posts/create。
    /// None = 尚未握手；现网前端凭 im:connection:established + 此值放行 pendingSends。
    pub connection_id: Option<String>,

    /// per-channel 聚合（消息 + 游标 + gate）
    pub channels: HashMap<ChannelId, Channel>,

    /// 待对账的发送（temporaryId → PendingSend）
    /// 来自源码：乐观落库后等 echo 或 sync 对账，15s 超时 → unSend
    pub pending_sends: HashMap<TemporaryId, PendingSend>,

    /// 统一 corr 回报路由表：corr → 一条在途异步操作的上下文（`CorrelationContext`）。
    ///
    /// 取代旧 4 张分散 HashMap（`corr_to_channel` / `corr_to_send` / `corr_to_sync` /
    /// `continuation_pending`）+ 1 个 `Option`（`scan_corr`）。`PortReply` 到达时
    /// 单次 `remove(corr)` 取出上下文，按变体 `match` 分发；一个 corr 恰对应一个变体，
    /// 类别互斥由数据结构（一 key 一 value）+ corr 全局唯一（见 `CorrelationContext` 不变量）保证。
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

    /// B2 冷启动自愈：本批次每个 increment_channel 帧带来的服务端权威水位
    /// （`inc.last_event_seq`，单调 max 累计）。
    ///
    /// 真源 cses-client `heal_incomplete_backfill_once`（post.rs:666-715）：冷启动时
    /// 「cursor 已推到 maxSeq 但本地只灌了首屏」的卡死 channel，靠「本地落后水位」识别后
    /// 补发 sync 全量回填。helix sans-IO 核无直接读本地消息数的能力，故等价信号为
    /// 「local cursor < 服务端 increment 水位」——记录于此。
    /// 与 `increment_fetched` 同生命周期（hello 起点 clear），防跨会话残留误判。
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
}

impl ImState {
    /// 分配下一个出站 WS 消息序号（从 1 开始单调递增）。
    pub fn next_client_seq(&mut self) -> u64 {
        self.client_seq += 1;
        self.client_seq
    }
}

impl ImState {
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
        }
    }

    /// 会话边界 reset（§7 跨会话残留）：每个 hello 窗口起点清空 increment 批次集，
    /// 避免上一会话残留让重连后对已追平 channel 误触发 sync 风暴。
    pub fn reset_increment_batch(&mut self) {
        self.increment_fetched.clear();
        self.need_sync_skip.clear();
        self.increment_target.clear();
        self.backfill_healed = false;
    }
}

impl Default for ImState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod id26_tests {
    use super::{test_channel_id, Id26};

    #[test]
    fn from_str_accepts_exactly_26_ascii_and_roundtrips() {
        let s = "5ffd465e263864787b87aa01"; // 24 字符——非法
        assert!(Id26::from_str(s).is_none(), "24 字符应拒绝");
        let s26 = "5ffd465e263864787b87aa0123"; // 26 字符
        assert_eq!(s26.len(), 26);
        let id = Id26::from_str(s26).expect("26 ASCII 应接受");
        assert_eq!(id.as_str(), s26, "as_str 必须原样回放");
    }

    #[test]
    fn from_str_rejects_wrong_len_and_non_ascii() {
        assert!(Id26::from_str("").is_none());
        assert!(Id26::from_str("abc").is_none()); // 太短
        assert!(Id26::from_str(&"a".repeat(27)).is_none()); // 太长
        assert!(Id26::from_str("你好你好你好你好你好你好你").is_none()); // 非 ASCII（字节数也≠26）
    }

    #[test]
    fn from_str_rejects_invalid_ascii_alphabet() {
        let invalid = [
            "5ffd465e263864787b87aa012!",
            "5ffd465e263864787b87aa012 ",
            "5ffd465e263864787b87aa012_",
            "5ffd465e263864787b87aa012A",
        ];
        for s in invalid {
            assert_eq!(s.len(), 26);
            assert!(Id26::from_str(s).is_none(), "must reject {s:?}");
        }
    }

    #[test]
    fn test_channel_id_is_26_alnum_and_distinct() {
        let a = test_channel_id(1);
        let b = test_channel_id(2);
        assert_eq!(a.as_str().len(), 26);
        assert_ne!(a, b);
        assert_eq!(a, test_channel_id(1), "确定性");
        // 经 from_str 再解析应等价（26 位 [a-z0-9] fixture）
        assert_eq!(super::ChannelId::from_str(a.as_str()), Some(a));
    }

    #[test]
    fn hash_u64_deterministic() {
        let a = test_channel_id(7);
        assert_eq!(a.hash_u64(), a.hash_u64());
        assert_ne!(test_channel_id(7).hash_u64(), test_channel_id(8).hash_u64());
    }
}
