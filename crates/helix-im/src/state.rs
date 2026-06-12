//! ImState — ImModule 的全量状态（纯数据，无 I/O 句柄）
//!
//! 对应源码实测的核心状态字段：
//! - channels: per-channel cursor + gate + buffer
//! - pending_sends: temporaryId → PendingSend（等待 echo/sync 对账）
//! - conn_state: WS 连接状态机
//! - 各种 corr 路由索引（corr_to_channel / corr_to_send / corr_to_sync）

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use std::collections::HashMap;

// ─── 值对象（全部 newtype，类型层禁止混淆）────────────────────────────────

/// 频道 ID（不透明 u64）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelId(pub u64);

/// Server 消息 ID（echo 到时的真实 ID）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerId(pub u64);

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
}

impl ImState {
    pub fn new() -> Self {
        Self {
            conn: ConnState::Disconnected,
            channels: HashMap::new(),
            pending_sends: HashMap::new(),
            corr_to_channel: HashMap::new(),
            corr_to_send: HashMap::new(),
            corr_to_sync: HashMap::new(),
            scan_corr: None,
            continuation_pending: HashMap::new(),
            backoff_timer: None,
            ping_timer: None,
        }
    }
}

impl Default for ImState {
    fn default() -> Self {
        Self::new()
    }
}
