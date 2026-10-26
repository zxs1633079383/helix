//! Native WebSocket thin shell over the shared host client.
//!
//! WS 协议执行（连接/收发/Text 出站/入站读任务）统一在 `helix-driver-host` 的
//! `SharedWsClient`（ADR-007 双端共享）。native 只保留预填表激活接缝与现网兼容构造器：
//! - **握手头**（`with_handshake_headers`）：现网 Go 在 WS 升级阶段按 `cookieId=userId` 建 session，
//!   session 空则不发 hello、~5s 超时关连接，故身份头必须随 upgrade 请求带（灌进共享 header registry）。
//! - **入站推送**（`SharedWsClient::with_inbound_tick`）：native engine_loop 从不调 `recv()`，
//!   入站帧全靠读任务作 `Tick::Inbound` 直发泵（A2 不变量）。

use crate::network_recovery::{NetworkAvailability, NetworkRecovery, NetworkRecoveryPolicy};
use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::FrameSender;
use helix_core::PortError;
use helix_core::Tick;
use helix_driver_host::{
    AsyncMetricSink, ConnectionEpochs, HostHeaderRegistry, HostNetworkConfig, NoopMetricSink,
    SharedWsClient, TickIngressSender, WsConnectionActivation,
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::{mpsc, watch};

mod ingress;
use ingress::NativeTickSender;

/// PC/Tauri transport wrapper.
///
/// WebSocket protocol execution and per-connection lifecycle live in `helix-driver-host`;
/// native keeps only compatibility constructors and the host-cli prefilled-table seam.
pub struct NativeTransport {
    inner: Option<SharedWsClient>,
    init_error: Option<String>,
    /// WS upgrade 握手头（如现网 Go 鉴权 `cookieId=userId`）。
    /// 现网 Go 在 HTTP 升级中间件按 header 建 session，session 空则不发 hello、超时关连接，
    /// 故身份头必须随 upgrade 请求带，而非仅 HTTP 客户端全局头。connect 前灌进共享 registry。
    handshake_headers: Vec<(String, String)>,
    reconnect: Option<NativeReconnectFactory>,
    connected_once: bool,
}

#[derive(Clone)]
pub(crate) struct NativeReconnectFactory {
    config: HostNetworkConfig,
    headers: HostHeaderRegistry,
    id: TransportId,
    tick_tx: Option<NativeTickSender>,
    metrics: Arc<dyn AsyncMetricSink>,
    /// 跨代共享的连接代际：新连接一建立就成为当前代，被替换连接收尾时静默退出。
    epochs: Arc<ConnectionEpochs>,
    pub(crate) recovery: Option<NetworkRecovery>,
    handshake_headers: Vec<(String, String)>,
    headers_initialized: Arc<AtomicBool>,
}

impl NativeReconnectFactory {
    /// Seeds cold-start headers once; reconnects retain later registry/auth updates.
    pub(crate) fn build(&self) -> Result<NativeTransport, PortError> {
        if !self.headers_initialized.load(Ordering::Acquire) {
            for (name, value) in &self.handshake_headers {
                self.headers.set_header_sync(name, value)?;
            }
            self.headers_initialized.store(true, Ordering::Release);
        }
        let mut inner = SharedWsClient::with_registry(self.config.clone(), self.headers.clone())?
            .with_metric_sink(Arc::clone(&self.metrics))
            .with_epoch(self.epochs.next(), Arc::clone(&self.epochs));
        if let Some(tx) = &self.tick_tx {
            inner = tx.attach(inner, self.id);
        }
        Ok(NativeTransport::from_shared_parts(
            inner,
            Some(self.clone()),
        ))
    }
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
                inner: None,
                init_error: Some(e.to_string()),
                handshake_headers: Vec::new(),
                reconnect: None,
                connected_once: false,
            },
        }
    }

    pub fn new_stamped(
        url: impl Into<String>,
        id: TransportId,
        tick_tx: Option<TickIngressSender>,
    ) -> Self {
        let config = HostNetworkConfig::new("", url.into());
        match Self::with_config_parts(config, id, tick_tx.map(NativeTickSender::Stamped)) {
            Ok(transport) => transport,
            Err(error) => Self {
                inner: None,
                init_error: Some(error.to_string()),
                handshake_headers: Vec::new(),
                reconnect: None,
                connected_once: false,
            },
        }
    }

    /// 注入 WS upgrade 握手头（builder）。现网 Go：`cookieId=userId`（+ 可选 companyId 等）。
    /// 头在 `connect` 前灌进 `SharedWsClient` 的共享 header registry，随 upgrade 请求带出。
    pub fn with_handshake_headers(mut self, headers: Vec<(String, String)>) -> Self {
        if let Some(factory) = &mut self.reconnect {
            factory.handshake_headers = headers.clone();
        }
        self.handshake_headers = headers;
        self
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        if let Some(inner) = self.inner.take() {
            self.inner = Some(inner.with_metric_sink(Arc::clone(&metrics)));
        }
        if let Some(factory) = &mut self.reconnect {
            factory.metrics = metrics;
        }
        self
    }

    pub fn with_config(
        config: HostNetworkConfig,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Result<Self, PortError> {
        Self::with_config_parts(config, id, tick_tx.map(NativeTickSender::Raw))
    }

    fn with_config_parts(
        config: HostNetworkConfig,
        id: TransportId,
        tick_tx: Option<NativeTickSender>,
    ) -> Result<Self, PortError> {
        let headers = HostHeaderRegistry::default();
        // 首连也属于第 1 代：被后续重建替换后，它的收尾同样不得发布 Disconnected。
        let epochs = Arc::new(ConnectionEpochs::default());
        let mut inner = SharedWsClient::with_registry(config.clone(), headers.clone())?
            .with_epoch(epochs.next(), Arc::clone(&epochs));
        if let Some(tx) = &tick_tx {
            inner = tx.attach(inner, id);
        }
        let reconnect = NativeReconnectFactory {
            config,
            headers,
            id,
            tick_tx,
            metrics: Arc::new(NoopMetricSink),
            epochs,
            recovery: None,
            handshake_headers: Vec::new(),
            headers_initialized: Arc::new(AtomicBool::new(false)),
        };
        Ok(Self::from_shared_parts(inner, Some(reconnect)))
    }

    /// Fallible stamped construction exposes configuration failures before starting a runtime.
    pub fn with_config_stamped(
        config: HostNetworkConfig,
        id: TransportId,
        tick_tx: Option<TickIngressSender>,
    ) -> Result<Self, PortError> {
        Self::with_config_parts(config, id, tick_tx.map(NativeTickSender::Stamped))
    }

    pub fn from_shared(
        inner: SharedWsClient,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        let inner = match tick_tx {
            Some(tx) => inner.with_inbound_tick(id, tx),
            None => inner,
        };
        Self::from_shared_parts(inner, None)
    }

    fn from_shared_parts(inner: SharedWsClient, reconnect: Option<NativeReconnectFactory>) -> Self {
        Self {
            inner: Some(inner),
            init_error: None,
            handshake_headers: Vec::new(),
            reconnect,
            connected_once: false,
        }
    }

    /// Enables cold-start recovery without blocking local engine/storage initialization.
    pub fn with_network_recovery(
        mut self,
        availability: watch::Receiver<NetworkAvailability>,
    ) -> Self {
        if let Some(factory) = &mut self.reconnect {
            factory.recovery = Some(NetworkRecovery {
                availability,
                policy: NetworkRecoveryPolicy::default(),
            });
        }
        self
    }

    /// Tests and isolated hosts can shorten delays without changing production defaults.
    pub fn with_network_recovery_policy(mut self, policy: NetworkRecoveryPolicy) -> Self {
        if let Some(recovery) = self
            .reconnect
            .as_mut()
            .and_then(|factory| factory.recovery.as_mut())
        {
            recovery.policy = policy;
        }
        self
    }

    /// A registered, unconnected transport needs one initial supervisor attempt.
    pub(crate) fn needs_initial_connection(&self) -> bool {
        !self.connected_once
    }

    pub(crate) fn reconnect_factory(&self) -> Option<NativeReconnectFactory> {
        self.reconnect.clone()
    }

    /// connect 路径用：状态机推进需 `&mut`（合法）。
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

    /// send/recv/close 稳态路径用：`SharedWsClient::{send,recv,close}` 已收紧 `&self`，
    /// 只需共享引用（与 FrameSender port `&self` 对齐，消除外层锁）。
    fn inner(&self) -> Result<&SharedWsClient, PortError> {
        match self.inner.as_ref() {
            Some(inner) => Ok(inner),
            None => Err(PortError::Transport(
                self.init_error
                    .clone()
                    .unwrap_or_else(|| "transport not initialized".to_string()),
            )),
        }
    }
}

impl NativeTransport {
    /// 完成握手并返回一次性激活令牌；此时 sender 已可用，但 reader 尚未放行。
    pub async fn connect(&mut self) -> Result<WsConnectionActivation, PortError> {
        // 握手头灌进共享 header registry：SharedWsClient::connect 会把 registry 快照
        // 逐条注入 WS upgrade 请求（现网 Go cookieId=userId 建 session 依赖此）。
        if !self.handshake_headers.is_empty() {
            let registry = self.inner_mut()?.headers();
            registry
                .replace_headers(self.handshake_headers.iter().map(|(k, v)| (k, v)))
                .await?;
            if let Some(factory) = &self.reconnect {
                factory.headers_initialized.store(true, Ordering::Release);
            }
        }

        let timeout = self
            .reconnect
            .as_ref()
            .filter(|factory| factory.recovery.is_some())
            .map(|factory| factory.config.timeout);
        let activation = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, self.inner_mut()?.connect())
                .await
                .map_err(|_| PortError::Transport("websocket handshake timeout".into()))??,
            None => self.inner_mut()?.connect().await?,
        };
        self.connected_once = true;
        Ok(activation)
    }

    async fn send_frame(&self, frame: Bytes) -> Result<(), PortError> {
        if self.reconnect.as_ref().and_then(|factory| factory.recovery.as_ref())
            .is_some_and(|recovery| recovery.availability.has_changed().is_ok() && *recovery.availability.borrow() == NetworkAvailability::Offline) {
            return Err(PortError::NetworkUnavailable);
        }
        // 出站走 SharedWsClient::send（已修为 Message::Text，现网 Go Text→JSON 解码）。
        self.inner()?.send(frame).await
    }

    pub async fn recv(&self) -> Result<Option<Bytes>, PortError> {
        // 生产 native 路径（tick_tx=Some）入站走 Tick::Inbound，engine_loop 不调本方法；
        // 此方法仅服务单测手动驱动路径（tick_tx=None → SharedWsClient frame_rx 拉取）。
        self.inner()?.recv().await
    }

    pub async fn close(&self) -> Result<(), PortError> {
        self.inner()?.close().await
    }
}

#[async_trait::async_trait]
impl FrameSender for NativeTransport {
    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        self.send_frame(frame).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::TransportId;

    /// A refreshed cookie must survive a factory rebuild instead of being replaced by startup headers.
    #[tokio::test]
    async fn reconnect_factory_preserves_updated_headers_after_cold_seed() {
        let transport = NativeTransport::new("ws://127.0.0.1:1", TransportId::from_raw(0), None)
            .with_handshake_headers(vec![("cookieId".into(), "old".into())]);
        let factory = transport.reconnect_factory().unwrap();
        let _first = factory.build().unwrap();
        factory
            .headers
            .set_header("cookieId", "updated")
            .await
            .unwrap();
        let rebuilt = factory.build().unwrap();
        assert!(rebuilt.handshake_headers.is_empty());
        assert_eq!(
            factory.headers.snapshot().await.get("cookieId").unwrap(),
            "updated"
        );
    }

    /// Business WS sends are rejected before touching the socket when the shared fact is offline.
    #[tokio::test]
    async fn confirmed_offline_ws_send_is_not_submitted() {
        let (_tx,rx)=tokio::sync::watch::channel(NetworkAvailability::Offline);
        let transport=NativeTransport::new("ws://127.0.0.1:1",TransportId::from_raw(0),None).with_network_recovery(rx);
        assert!(matches!(transport.send(Bytes::from_static(b"write")).await,Err(PortError::NetworkUnavailable)));
    }

    #[tokio::test]
    async fn test_send_on_disconnected_returns_err() {
        let (tx, _rx) = mpsc::channel(8);
        let transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(1), Some(tx));

        let result = transport.send(Bytes::from_static(b"hello")).await;
        assert!(matches!(result, Err(PortError::Transport(_))));
    }

    #[tokio::test]
    async fn test_recv_on_disconnected_returns_none() {
        let (tx, _rx) = mpsc::channel(8);
        let transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(2), Some(tx));

        let result = transport.recv().await.expect("recv should not error");
        assert!(result.is_none(), "disconnected recv should return None");
    }

    #[tokio::test]
    async fn test_close_on_disconnected_is_ok() {
        let (tx, _rx) = mpsc::channel(8);
        let transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(3), Some(tx));
        transport.close().await.ok();
    }
}
