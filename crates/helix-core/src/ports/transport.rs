//! Transport port — WS 连接抽象
//!
//! ## 三端实现（ACL-2）
//! - native: `helix-driver-native` NativeTransport（tokio-tungstenite）
//! - wasm:   `helix-driver-wasm` WasmTransport（ws_stream_wasm / web-sys）
//! - ffi:    `helix-driver-ffi` FfiTransport（C ABI 回调）

use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};
use bytes::Bytes;

/// WebSocket 连接的 core 抽象。
///
/// ## 对象安全
///
/// 返回 `Result<_, PortError>` 而非关联类型，确保 dyn-safe。
/// driver 用 thiserror 翻译具体 crate 错误到 `PortError`。
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport: MaybeSend + MaybeSync + 'static {
    /// 建立 WebSocket 连接。
    ///
    /// ## 为什么唯独 connect 保留 `&mut self`
    ///
    /// connect 是状态机推进（Disconnected → Connected），driver 实现（如 native
    /// `NativeTransport`）在此真改自身状态字段（connected/disconnected 标记）——这是合法的
    /// `&mut self` 用途。把它收紧成 `&self` 会逼实现把这些字段翻成 `Cell/AtomicBool` 内部可变，
    /// blast 扩大且无收益（connect 本就应在表外被独占持有时调用，无并发争用）。
    async fn connect(&mut self) -> Result<(), PortError>;

    /// 发送一帧（driver 处理分片、masking 等协议细节）。
    ///
    /// ## 为什么 `&self`（稳态方法收紧 / fix/lifecycle-net-decouple 后续）
    ///
    /// send/recv/close 是连接**稳态**下的并发安全操作：driver 内部用 `Arc<Mutex<状态>>`
    /// 串行化（如 host `SharedWsClient::state`），语义上只需共享引用。改 `&self` 让 host 把
    /// transport 以 `Arc<Tr>`（无外层 `Mutex`）存进路由表，`Effect::Send` 分流走 `t.send()` 即可，
    /// 消除「外层锁跨 `connect().await`」这一架构脆弱点（连接期句柄经注册通道在连成功后才进表 →
    /// 连接期类型层不可见，而非靠运行时时序不变量保证不阻塞）。
    async fn send(&self, frame: Bytes) -> Result<(), PortError>;

    /// 接收下一帧（None = 连接已关闭）。`&self` 同 [`Transport::send`]。
    async fn recv(&self) -> Result<Option<Bytes>, PortError>;

    /// 关闭连接（发送 Close 帧）。`&self` 同 [`Transport::send`]。
    async fn close(&self) -> Result<(), PortError>;
}
