//! Transport port — WS 连接抽象
//!
//! ## 三端实现（ACL-2）
//! - native: `helix-driver-native` NativeTransport（tokio-tungstenite）
//! - wasm:   `helix-driver-wasm` WasmTransport（ws_stream_wasm / web-sys）
//! - ffi:    `helix-driver-ffi` FfiTransport（C ABI 回调）

use bytes::Bytes;
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// WebSocket 连接的 core 抽象。
///
/// ## 对象安全
///
/// 返回 `Result<_, PortError>` 而非关联类型，确保 dyn-safe。
/// driver 用 thiserror 翻译具体 crate 错误到 `PortError`。
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport: MaybeSend + MaybeSync + 'static {
    /// 建立 WebSocket 连接
    async fn connect(&mut self) -> Result<(), PortError>;

    /// 发送一帧（driver 处理分片、masking 等协议细节）
    async fn send(&mut self, frame: Bytes) -> Result<(), PortError>;

    /// 接收下一帧（None = 连接已关闭）
    async fn recv(&mut self) -> Result<Option<Bytes>, PortError>;

    /// 关闭连接（发送 Close 帧）
    async fn close(&mut self) -> Result<(), PortError>;
}
