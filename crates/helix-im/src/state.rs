//! ImState — ImModule 的全量纯数据状态；不持 I/O 句柄。

use crate::channel::Channel;
use crate::pending_send::PendingSend;
use crate::send::upload_props::PendingUpload;
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
// 外提 sibling `correlation.rs`（守结构闸 baseline + 给 reclaim 新增变体留余量；
// drift-review baseline 棘轮后门收敛）。re-export → `crate::state::CorrelationContext` 调用点零改。
#[path = "correlation.rs"]
mod correlation;
pub use correlation::CorrelationContext;

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

    /// UC-10：本 hello 会话累积 mention+urgent post id，global increment-end 拉 queryTodoList 后清空。
    pub about_me_post_ids: Vec<String>,

    /// hello 早于 cursor scan 回包时，首个 `/channels/load/increment` 只能带空 cursors。
    /// scan 回包加载持久 cursor 后需补发一次 bootstrap increment，避免冷启动只走 sync/notify
    /// 而缺失 `increment_channel` / `im:channel:increment`。
    pub pending_increment_bootstrap_after_scan: bool,

    /// UC-4.5 单频道 hydration 已完成 channel/member 落库、正在等待 sync 终态的 channel。
    /// terminal sync commit/no_change 后移除并触发本地 dialog/messages 读回投影。
    pub hydration_pending: HashSet<ChannelId>,
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
            pending_increment_bootstrap_after_scan: false,
            hydration_pending: HashSet::new(),
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
        self.pending_increment_bootstrap_after_scan = false;
        self.hydration_pending.clear();
        self.viewer_channel_roles.clear();
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
