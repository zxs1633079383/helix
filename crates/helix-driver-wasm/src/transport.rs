//! WasmTransport — archived web-sys WebSocket skeleton.
//!
//! This historical crate is not the product Web driver. It only follows the
//! current send-only core port so deleting the old lifecycle port does not
//! leave a stale contract behind.

use bytes::Bytes;
use helix_core::PortError;

pub struct WasmTransport {
    url: String,
    // TODO: ws: Option<web_sys::WebSocket>,
    // TODO: recv_rx: futures_channel::mpsc::Receiver<Bytes>,
}

impl WasmTransport {
    pub fn new(url: impl Into<String>) -> Self {
        Self { url: url.into() }
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn connect(&mut self) -> Result<(), PortError> {
        // TODO: browser-owned lifecycle adapter creates and wires WebSocket.
        let _ = &self.url;
        todo!("WasmTransport::connect")
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn recv(&self) -> Result<Option<Bytes>, PortError> {
        todo!("WasmTransport::recv")
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn close(&self) -> Result<(), PortError> {
        todo!("WasmTransport::close")
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl helix_core::ports::FrameSender for WasmTransport {
    async fn send(&self, _frame: Bytes) -> Result<(), PortError> {
        // TODO: self.ws.send_with_u8_array(&frame)
        todo!("WasmTransport::send")
    }
}
