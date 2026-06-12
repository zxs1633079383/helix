//! # tick.rs
//!
//! `Tick` 是喂给 `ExecutionShell::step()` 的确定性输入的 tagged union。
//!
//! ## 设计要点
//!
//! - 统一类型贯穿：`accepts(&Tick)` 和 `handle(&Tick)` 使用同一类型，
//!   消除原草稿中的 `InboundFrame` 中间层（审查意见2 P1 落实）
//! - `now_ms` 是 `step()` 的独立参数，不进 `Tick`——时间是外部注入的 Clock 读数
//! - `PortReply` 和 `Timer` 不走 `accepts()` 路由，由 `corr_map` 定向投递
//!   （审查意见2 P3 落实）
//! - BLK-2 修复：`InboundBytes` 和 `AppCommand` 改用 `bytes::Bytes` / `Cow<'static, str>`，
//!   消除 `'a` lifetime 污染，使 `Tick` 满足 `'static + Send`，
//!   从而 driver 的 `tokio::sync::mpsc::Sender<Tick>` 和 timer channel 可以正常工作。

use crate::effect::{Correlation, TimerId, TransportId};
use std::borrow::Cow;

/// 入站 wire bytes（未解析，由模块的 parser 解析为强类型事件）
///
/// 使用 `bytes::Bytes`（引用计数，O(1) clone），满足 `'static + Send`，
/// 可跨 await 点和 channel 传递。driver 从 WS buffer 创建 Bytes 时零拷贝。
#[derive(Debug, Clone)]
pub struct InboundBytes(pub bytes::Bytes);

impl InboundBytes {
    /// 从静态字节切片创建（主要用于测试）
    pub fn from_static(s: &'static [u8]) -> Self {
        Self(bytes::Bytes::from_static(s))
    }

    /// 返回内部字节切片引用（方便模块 parser 接收 &[u8]）
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// 端口响应结果
#[derive(Debug, Clone)]
pub enum PortOutcome {
    /// 端口操作成功，携带响应字节（可为空）
    Ok(ReplyBytes),
    /// 端口操作失败，携带分类错误
    Err(PortError),
}

/// 响应字节（零拷贝包装）
#[derive(Debug, Default, Clone)]
pub struct ReplyBytes(pub bytes::Bytes);

/// 端口错误（分类，支持精细退避策略）
///
/// MIN-3 修复：将裸 `PortErrCode(u32)` 替换为分类 enum，
/// IM 模块可区分 4xx/5xx/超时，实现差异化重试策略。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// 请求超时（网络层）
    Timeout,
    /// HTTP 错误，携带状态码（如 4xx、5xx）
    Http(u16),
    /// 存储错误，携带存储层错误码
    Storage(u32),
    /// 网络不可达
    Network,
    /// 其他未分类错误
    Other(u32),
}

/// 应用命令（UI 发来的命令）
///
/// BLK-2 修复：`payload` 改为 `bytes::Bytes`（引用计数零拷贝），
/// 消除 lifetime 'a，使 `Tick` 满足 `'static + Send`。
#[derive(Debug, Clone)]
pub struct AppCommand {
    /// 命令名称（静态字符串或 Cow，如 "send_message" / "read_channel"）
    pub name: Cow<'static, str>,
    /// 命令载荷（已序列化字节，由模块的 ACL-1 解析）
    pub payload: bytes::Bytes,
}

impl AppCommand {
    /// 从静态名称和字节切片创建（主要用于测试）
    pub fn new(name: &'static str, payload: impl Into<bytes::Bytes>) -> Self {
        Self {
            name: Cow::Borrowed(name),
            payload: payload.into(),
        }
    }
}

/// `ExecutionShell::step()` 的唯一输入类型。
///
/// ## BLK-2 修复说明
///
/// 原 `Tick<'a>` 含 `InboundBytes<'a>(&'a [u8])` 和 `AppCommand<'a>{ payload: &'a [u8] }`，
/// 无法满足 `tokio::sync::mpsc::Sender<Tick>` 的 `'static` 要求。
/// 修复：改为 `bytes::Bytes`（引用计数，O(1) clone，`'static + Send`）。
///
/// ## 路由规则（重要）
///
/// - `Inbound` / `Command`：通过 `Module::accepts()` 线性扫描路由到模块
/// - `PortReply`：通过 `ExecutionShell::corr_map` 定向投递（O(1) HashMap 查找）
/// - `Timer`：通过 `ExecutionShell::timer_map` 定向投递
///
/// 这确保多模块并存时，PortReply/Timer 不会被错误模块的 `accepts()` 误吞。
#[derive(Debug, Clone)]
pub enum Tick {
    /// 入站 wire 帧（WS 收到的字节）
    Inbound(InboundBytes),

    /// 端口操作完成回报（Persist / Http 的异步结果）
    /// corr_map 保证此 Tick 定向投递给发出对应 Correlation 的模块
    PortReply {
        corr: Correlation,
        outcome: PortOutcome,
    },

    /// 定时器触发（ScheduleTimer 到期后由 driver TimerRegistry 产生）
    /// timer_map 保证此 Tick 定向投递给 arm 了该 timer 的模块
    Timer(TimerId),

    /// 应用命令（UI 层通过 Tauri command / FFI / WASM JS 注入）
    Command(AppCommand),

    /// 传输连接建立（driver Transport 状态机重连成功后产生；内容路由，
    /// 模块据此执行业务策略——如 helix-im 的 cursor reset / proactive resync）
    Connected(TransportId),

    /// 传输连接断开（driver 私有状态机负责重连；模块只感知状态，不管退避）
    Disconnected(TransportId),
}
