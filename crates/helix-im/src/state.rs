//! ImState — ImModule 的全量状态（纯数据，无 I/O 句柄）
//!
//! 对应源码实测的核心状态字段：
//! - channels: per-channel cursor + gate + buffer
//! - pending_sends: temporaryId → PendingSend（等待 echo/sync 对账）
//! - conn_state: WS 连接状态机
//! - 各种 corr 路由索引（corr_to_channel / corr_to_send / corr_to_sync）

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use std::collections::{HashMap, HashSet};

// ─── 值对象（全部 newtype，类型层禁止混淆）────────────────────────────────

/// mattermost 风格定长 ID（26 字符 base32 ASCII）。
///
/// Copy newtype，零分配热路径 key（HX-C005）；真 Go channelId/userId/msgId 恒为 26 char。
/// 不透明：构造只经 `from_str`（边界零信任）/ `from_test_u64`（测试桥接）。ADR-009。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id26([u8; 26]);

impl Id26 {
    /// 边界零信任构造：仅当 `s` 恰 26 字节 ASCII 时 Some，否则 None（不 panic）。
    pub fn from_str(s: &str) -> Option<Self> {
        let b = s.as_bytes();
        if b.len() != 26 || !b.is_ascii() {
            return None;
        }
        let mut arr = [0u8; 26];
        arr.copy_from_slice(b);
        Some(Self(arr))
    }

    /// 零拷贝取字符串视图。
    pub fn as_str(&self) -> &str {
        // SAFETY: 不变量——Id26 只能经 from_str（已验证恰 26 字节 ASCII）或
        // from_test_u64（format!("{:026}") 全 ASCII 数字）构造，故内部恒为合法 UTF-8。
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

    /// 测试桥接：数值 → 26 字符确定性 id（零填充十进制）。test-only（注释约束，非编译期）。
    pub fn from_test_u64(n: u64) -> Self {
        let mut arr = [0u8; 26];
        arr.copy_from_slice(format!("{:026}", n).as_bytes());
        Self(arr)
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
    /// 测试桥接：数值 → 26 字符 ChannelId。test-only。
    pub fn from_test_u64(n: u64) -> Self {
        Self(Id26::from_test_u64(n))
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
    /// 测试桥接：数值 → 26 字符 ServerId。test-only。
    pub fn from_test_u64(n: u64) -> Self {
        Self(Id26::from_test_u64(n))
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

    /// MAJ-1 修复：路由索引（corr → channel_id），用于 channel sync Persist 回报。
    ///
    /// - `corr_to_channel`：仅存路由信息（corr → channel_id），不存 seq
    /// - `Channel.pending_commits`：持有 `corr → Seq`，是 cursor 推进的唯一聚合根
    /// - 路径：PortReply → `corr_to_channel.remove(corr)` → `channel.on_persist_ok(corr, fx)`
    pub corr_to_channel: HashMap<helix_core::Correlation, ChannelId>,

    /// 路由索引（corr → temporary_id），用于 P1 乐观落库 Persist 回报。
    pub corr_to_send: HashMap<helix_core::Correlation, TemporaryId>,

    /// 路由索引（corr → channel_id），用于 sync/notify Http 回报。
    pub corr_to_sync: HashMap<helix_core::Correlation, ChannelId>,

    /// on_start 时发出的 Scan corr（用于 proactive resync 路由）
    pub scan_corr: Option<helix_core::Correlation>,

    /// E3 续拉：Persist corr → channel_id（persist_corr 落库成功后触发续拉 sync Http）
    ///
    /// 仅当该批 sync 响应 `needs_continuation=true` 时才注册。
    /// PortReply Ok 路由 1 完成 on_persist_ok 后检查此 map，若命中则发续拉 Http。
    pub continuation_pending: HashMap<helix_core::Correlation, ChannelId>,

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
}

impl ImState {
    pub fn new() -> Self {
        Self {
            conn: ConnState::Disconnected,
            connection_id: None,
            channels: HashMap::new(),
            pending_sends: HashMap::new(),
            corr_to_channel: HashMap::new(),
            corr_to_send: HashMap::new(),
            corr_to_sync: HashMap::new(),
            scan_corr: None,
            continuation_pending: HashMap::new(),
            backoff_timer: None,
            ping_timer: None,
            increment_fetched: HashSet::new(),
            need_sync_skip: HashSet::new(),
        }
    }

    /// 会话边界 reset（§7 跨会话残留）：每个 hello 窗口起点清空 increment 批次集，
    /// 避免上一会话残留让重连后对已追平 channel 误触发 sync 风暴。
    pub fn reset_increment_batch(&mut self) {
        self.increment_fetched.clear();
        self.need_sync_skip.clear();
    }
}

impl Default for ImState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod id26_tests {
    use super::{ChannelId, Id26};

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
    fn from_test_u64_is_26_ascii_and_distinct() {
        let a = ChannelId::from_test_u64(1);
        let b = ChannelId::from_test_u64(2);
        assert_eq!(a.as_str().len(), 26);
        assert_ne!(a, b);
        assert_eq!(a, ChannelId::from_test_u64(1), "确定性");
        // 经 from_str 再解析应等价（26 ASCII 数字）
        assert_eq!(ChannelId::from_str(a.as_str()), Some(a));
    }

    #[test]
    fn hash_u64_deterministic() {
        let a = ChannelId::from_test_u64(7);
        assert_eq!(a.hash_u64(), a.hash_u64());
        assert_ne!(
            ChannelId::from_test_u64(7).hash_u64(),
            ChannelId::from_test_u64(8).hash_u64()
        );
    }
}
