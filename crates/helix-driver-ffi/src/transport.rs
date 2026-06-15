//! FfiTransport — C ABI 回调包装 + futures-channel 桥接

use bytes::Bytes;
use helix_core::PortError;

pub struct FfiTransport {
    // TODO: send_fn: extern "C" fn(*const u8, usize),
    // TODO: recv_rx: futures_channel::mpsc::Receiver<Bytes>,
    //       recv_rx 由宿主调用 helix_ws_on_recv 时写入
}

#[async_trait::async_trait]
impl helix_core::ports::Transport for FfiTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        // FFI driver 的"连接"由宿主控制，此处通知宿主可以开始
        todo!("FfiTransport::connect")
    }

    async fn send(&mut self, _frame: Bytes) -> Result<(), PortError> {
        // TODO: (self.send_fn)(frame.as_ptr(), frame.len());
        todo!("FfiTransport::send")
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        // TODO: self.recv_rx.next().await.ok_or(PortError::Transport("closed".into()))
        todo!("FfiTransport::recv")
    }

    async fn close(&mut self) -> Result<(), PortError> {
        todo!("FfiTransport::close")
    }
}
