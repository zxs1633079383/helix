//! WasmTransport — web-sys WebSocket（+ futures-channel 手桥）
//!
//! 优先使用 web-sys WebSocket + futures-channel 手桥（不依赖 ws_stream_wasm）。
//! ws_stream_wasm 可作备选，更换时只改此文件，port 契约不变。

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
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl helix_core::ports::Transport for WasmTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        // TODO: web_sys::WebSocket::new(&self.url)
        //       注册 onmessage / onerror / onclose 回调，
        //       回调里把帧 push 到 futures_channel::mpsc::Sender<Bytes>
        todo!("WasmTransport::connect")
    }

    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        // TODO: self.ws.send_with_u8_array(&frame)
        todo!("WasmTransport::send")
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        // TODO: self.recv_rx.next().await
        todo!("WasmTransport::recv")
    }

    async fn close(&mut self) -> Result<(), PortError> {
        // TODO: self.ws.close()
        todo!("WasmTransport::close")
    }
}
