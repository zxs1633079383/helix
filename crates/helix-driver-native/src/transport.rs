//! NativeTransport — tokio-tungstenite WebSocket

use helix_core::PortError;
use helix_core::ports::Transport;
use bytes::Bytes;

pub struct NativeTransport {
    url: String,
    // TODO: inner: Option<tokio_tungstenite::WebSocketStream<...>>
}

impl NativeTransport {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }
}

// 错误翻译（ACL-2）
fn map_ws_err(e: impl std::fmt::Display) -> PortError {
    PortError::Transport(e.to_string())
}

#[async_trait::async_trait]
impl Transport for NativeTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        // TODO: tokio_tungstenite::connect_async(&self.url)
        todo!("NativeTransport::connect")
    }

    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        // TODO: self.inner.as_mut().unwrap().send(Message::Binary(frame))
        todo!("NativeTransport::send")
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        // TODO: self.inner.as_mut().unwrap().next().await
        todo!("NativeTransport::recv")
    }

    async fn close(&mut self) -> Result<(), PortError> {
        // TODO: self.inner.as_mut().unwrap().close(None)
        todo!("NativeTransport::close")
    }
}
