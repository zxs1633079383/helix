//! Native WebSocket thin shell over the shared host client.

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::PortError;
use helix_core::Tick;
use helix_driver_host::{HostNetworkConfig, SharedWsClient};
use tokio::sync::mpsc;

/// PC/Tauri transport wrapper.
///
/// WebSocket protocol execution lives in `helix-driver-host`; native keeps only
/// engine-loop lifecycle tick wiring and compatibility constructors.
pub struct NativeTransport {
    id: TransportId,
    inner: Option<SharedWsClient>,
    init_error: Option<String>,
    tick_tx: Option<mpsc::Sender<Tick>>,
    connected: bool,
    disconnected_emitted: bool,
}

impl NativeTransport {
    pub fn new(
        url: impl Into<String>,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        let config = HostNetworkConfig::new("", url.into());
        match Self::with_config(config, id, tick_tx) {
            Ok(transport) => transport,
            Err(e) => Self {
                id,
                inner: None,
                init_error: Some(e.to_string()),
                tick_tx: None,
                connected: false,
                disconnected_emitted: false,
            },
        }
    }

    pub fn with_config(
        config: HostNetworkConfig,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Result<Self, PortError> {
        SharedWsClient::new(config).map(|inner| Self::from_shared(inner, id, tick_tx))
    }

    pub fn from_shared(
        inner: SharedWsClient,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        Self {
            id,
            inner: Some(inner),
            init_error: None,
            tick_tx,
            connected: false,
            disconnected_emitted: false,
        }
    }

    async fn emit(&self, tick: Tick) {
        if let Some(tx) = &self.tick_tx {
            tx.send(tick).await.ok();
        }
    }

    async fn emit_disconnected_once(&mut self) {
        if self.connected && !self.disconnected_emitted {
            self.emit(Tick::Disconnected(self.id)).await;
            self.disconnected_emitted = true;
            self.connected = false;
        }
    }

    fn inner_mut(&mut self) -> Result<&mut SharedWsClient, PortError> {
        match self.inner.as_mut() {
            Some(inner) => Ok(inner),
            None => Err(PortError::Transport(
                self.init_error
                    .clone()
                    .unwrap_or_else(|| "transport not initialized".to_string()),
            )),
        }
    }
}

#[async_trait::async_trait]
impl Transport for NativeTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        self.inner_mut()?.connect().await?;
        self.connected = true;
        self.disconnected_emitted = false;
        self.emit(Tick::Connected(self.id)).await;
        Ok(())
    }

    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        self.inner_mut()?.send(frame).await
    }

    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        let result = self.inner_mut()?.recv().await?;
        if result.is_none() {
            self.emit_disconnected_once().await;
        }
        Ok(result)
    }

    async fn close(&mut self) -> Result<(), PortError> {
        let result = self.inner_mut()?.close().await;
        self.emit_disconnected_once().await;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::TransportId;

    #[tokio::test]
    async fn test_send_on_disconnected_returns_err() {
        let (tx, _rx) = mpsc::channel(8);
        let mut transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(1), Some(tx));

        let result = transport.send(Bytes::from_static(b"hello")).await;
        assert!(matches!(result, Err(PortError::Transport(_))));
    }

    #[tokio::test]
    async fn test_recv_on_disconnected_returns_none() {
        let (tx, _rx) = mpsc::channel(8);
        let mut transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(2), Some(tx));

        let result = transport.recv().await.expect("recv should not error");
        assert!(result.is_none(), "disconnected recv should return None");
    }

    #[tokio::test]
    async fn test_close_on_disconnected_is_ok() {
        let (tx, _rx) = mpsc::channel(8);
        let mut transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(3), Some(tx));
        transport.close().await.ok();
    }
}
