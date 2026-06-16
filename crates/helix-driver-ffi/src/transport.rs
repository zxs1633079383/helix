//! FFI transport — shared WS product path plus host-network fallback.

use std::sync::Arc;

use bytes::Bytes;
use futures_channel::mpsc;
use futures_util::StreamExt;
use helix_core::ports::Transport;
use helix_core::PortError;
use helix_driver_host::SharedWsClient;

use crate::callbacks::HelixCallbacks;

pub struct FfiTransport {
    mode: FfiTransportMode,
}

enum FfiTransportMode {
    Shared(SharedWsClient),
    Callback {
        callbacks: Arc<HelixCallbacks>,
        inbound_rx: mpsc::UnboundedReceiver<Bytes>,
    },
}

impl FfiTransport {
    pub fn new(shared: SharedWsClient) -> Self {
        Self {
            mode: FfiTransportMode::Shared(shared),
        }
    }

    pub fn from_callbacks(callbacks: Arc<HelixCallbacks>) -> (Self, mpsc::UnboundedSender<Bytes>) {
        let (inbound_tx, inbound_rx) = mpsc::unbounded();
        (
            Self {
                mode: FfiTransportMode::Callback {
                    callbacks,
                    inbound_rx,
                },
            },
            inbound_tx,
        )
    }
}

#[async_trait::async_trait]
impl Transport for FfiTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        match &mut self.mode {
            FfiTransportMode::Shared(shared) => shared.connect().await,
            FfiTransportMode::Callback { .. } => Ok(()),
        }
    }

    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        match &mut self.mode {
            FfiTransportMode::Shared(shared) => shared.send(frame).await,
            FfiTransportMode::Callback { callbacks, .. } => {
                let Some(send_ws_frame) = callbacks.send_ws_frame else {
                    return Err(PortError::Transport(
                        "host ws_send fallback callback missing".to_string(),
                    ));
                };
                let status = send_ws_frame(callbacks.ctx, frame.as_ptr(), frame.len());
                if status == 0 {
                    Ok(())
                } else {
                    Err(PortError::Transport(format!(
                        "host ws_send fallback failed: {status}"
                    )))
                }
            }
        }
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        match &mut self.mode {
            FfiTransportMode::Shared(shared) => shared.recv().await,
            FfiTransportMode::Callback { inbound_rx, .. } => Ok(inbound_rx.next().await),
        }
    }

    async fn close(&mut self) -> Result<(), PortError> {
        match &mut self.mode {
            FfiTransportMode::Shared(shared) => shared.close().await,
            FfiTransportMode::Callback { inbound_rx, .. } => {
                inbound_rx.close();
                Ok(())
            }
        }
    }
}
