//! ImState — ImModule 的全量状态（纯数据，无 I/O 句柄）
//!
//! 对应源码实测的核心状态字段：
//! - channels: per-channel cursor + gate + buffer
//! - pending_sends: temporaryId → PendingSend（等待 echo/sync 对账）
//! - pending_commits: corr → target_seq（写成功才推 cursor，peek/commit 两段式）
//! - conn_state: WS 连接状态机

use std::collections::HashMap;
use crate::channel::Channel;
use crate::pending_send::PendingSend;

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

    /// MAJ-1 修复：路由索引（corr → channel_id），替代原 pending_commits。
    ///
    /// 原 `pending_commits: HashMap<Correlation, (ChannelId, Seq)>` 与
    /// `Channel.pending_commits: HashMap<Correlation, Seq>` 是两份独立 source of truth，
    /// 导致 cursor 推进逻辑分裂（module.rs 查 state，channel.rs 查自身，互不调用）。
    ///
    /// 修复后：
    /// - `corr_to_channel`：仅存路由信息（corr → channel_id），不存 seq
    /// - `Channel.pending_commits`：持有 `corr → Seq`，是 cursor 推进的唯一聚合根
    /// - 路径：PortReply → `corr_to_channel.remove(corr)` → `channel.on_persist_ok(corr, fx)`
    pub corr_to_channel: HashMap<helix_core::Correlation, ChannelId>,

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
