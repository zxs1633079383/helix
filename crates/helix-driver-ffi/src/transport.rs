//! FFI transport — C++ 直连模型，Rust 持有 SharedWsClient。
//!
//! Dart-host 的 Callback 模式（`FfiTransportMode::Callback`）已删除：
//! C++ 直连模型下 Rust 自持 WS，host 不喂帧，不需要 inbound channel 注入路径。

use bytes::Bytes;
use helix_core::ports::Transport;
use helix_core::PortError;
use helix_driver_host::SharedWsClient;

/// FFI transport 实现，仅 Shared 模式（Rust 持有 SharedWsClient）。
pub struct FfiTransport {
    shared: SharedWsClient,
}

impl FfiTransport {
    pub fn new(shared: SharedWsClient) -> Self {
        Self { shared }
    }
}

#[async_trait::async_trait]
impl Transport for FfiTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        self.shared.connect().await
    }

    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        self.shared.send(frame).await
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        self.shared.recv().await
    }

    async fn close(&mut self) -> Result<(), PortError> {
        self.shared.close().await
    }
}
