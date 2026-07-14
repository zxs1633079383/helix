//! Native WebSocket thin shell over the shared host client.
//!
//! WS 协议执行（连接/收发/Text 出站/入站读任务）统一在 `helix-driver-host` 的
//! `SharedWsClient`（ADR-007 双端共享）。native 只保留 engine-loop 生命周期 Tick 接线
//! 与现网兼容构造器：
//! - **握手头**（`with_handshake_headers`）：现网 Go 在 WS 升级阶段按 `cookieId=userId` 建 session，
//!   session 空则不发 hello、~5s 超时关连接，故身份头必须随 upgrade 请求带（灌进共享 header registry）。
//! - **入站推送**（`SharedWsClient::with_inbound_tick`）：native engine_loop 从不调 `recv()`，
//!   入站帧全靠读任务作 `Tick::Inbound` 直发泵（A2 不变量）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::PortError;
use helix_core::Tick;
use helix_driver_host::{
    AsyncMetricSink, HostHeaderRegistry, HostNetworkConfig, NoopMetricSink, SharedWsClient,
    TickIngressSender,
};
use tokio::sync::mpsc;

mod ingress;
use ingress::NativeTickSender;

/// PC/Tauri transport wrapper.
///
/// WebSocket protocol execution lives in `helix-driver-host`; native keeps only
/// engine-loop lifecycle tick wiring and compatibility constructors.
pub struct NativeTransport {
    id: TransportId,
    inner: Option<SharedWsClient>,
    init_error: Option<String>,
    tick_tx: Option<NativeTickSender>,
    /// 连接生命周期标记（内部可变）。
    ///
    /// send/recv/close 已收紧为 `&self`（Transport port，消除外层锁跨 await）；recv/close 路径仍需
    /// 推进「已连接 / 已发过 Disconnected」状态做幂等去重 → 用 `AtomicBool` 内部可变（单线程
    /// current_thread/native 单 transport 下无真并发，`Ordering::Relaxed` 足够）。connect 仍 `&mut self`
    /// 可直写，但统一走原子设值保一致。
    connected: AtomicBool,
    disconnected_emitted: AtomicBool,
    /// WS upgrade 握手头（如现网 Go 鉴权 `cookieId=userId`）。
    /// 现网 Go 在 HTTP 升级中间件按 header 建 session，session 空则不发 hello、超时关连接，
    /// 故身份头必须随 upgrade 请求带，而非仅 HTTP 客户端全局头。connect 前灌进共享 registry。
    handshake_headers: Vec<(String, String)>,
    reconnect: Option<NativeReconnectFactory>,
}

#[derive(Clone)]
pub(crate) struct NativeReconnectFactory {
    config: HostNetworkConfig,
    headers: HostHeaderRegistry,
    id: TransportId,
    tick_tx: Option<NativeTickSender>,
    metrics: Arc<dyn AsyncMetricSink>,
}

impl NativeReconnectFactory {
    pub(crate) fn build(&self) -> Result<NativeTransport, PortError> {
        let mut inner = SharedWsClient::with_registry(self.config.clone(), self.headers.clone())?
            .with_metric_sink(Arc::clone(&self.metrics));
        if let Some(tx) = &self.tick_tx {
            inner = tx.attach(inner, self.id);
        }
        Ok(NativeTransport::from_shared_parts(
            inner,
            self.id,
            self.tick_tx.clone(),
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
                id,
                inner: None,
                init_error: Some(e.to_string()),
                tick_tx: None,
                connected: AtomicBool::new(false),
                disconnected_emitted: AtomicBool::new(false),
                handshake_headers: Vec::new(),
                reconnect: None,
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
            Err(e) => Self {
                id,
                inner: None,
                init_error: Some(e.to_string()),
                tick_tx: None,
                connected: AtomicBool::new(false),
                disconnected_emitted: AtomicBool::new(false),
                handshake_headers: Vec::new(),
                reconnect: None,
            },
        }
    }

    /// 注入 WS upgrade 握手头（builder）。现网 Go：`cookieId=userId`（+ 可选 companyId 等）。
    /// 头在 `connect` 前灌进 `SharedWsClient` 的共享 header registry，随 upgrade 请求带出。
    pub fn with_handshake_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.handshake_headers = headers;
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
        let mut inner = SharedWsClient::with_registry(config.clone(), headers.clone())?;
        // A2：tick_tx=Some（生产 native 路径）→ 共享读任务作 Tick::Inbound 直发泵；
        // tick_tx=None（单测手动驱动）→ 不注入，读任务回退 frame_rx，由 recv() 拉取。
        if let Some(tx) = &tick_tx {
            inner = tx.attach(inner, id);
        }
        let reconnect = NativeReconnectFactory {
            config,
            headers,
            id,
            tick_tx: tick_tx.clone(),
            metrics: Arc::new(NoopMetricSink),
        };
        Ok(Self::from_shared_parts(inner, id, tick_tx, Some(reconnect)))
    }

    pub fn from_shared(
        inner: SharedWsClient,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        Self::from_shared_parts(inner, id, tick_tx.map(NativeTickSender::Raw), None)
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        if let Some(inner) = self.inner.take() {
            self.inner = Some(inner.with_metric_sink(Arc::clone(&metrics)));
        }
        if let Some(factory) = &mut self.reconnect {
            factory.metrics = Arc::clone(&metrics);
        }
        self
    }

    fn from_shared_parts(
        inner: SharedWsClient,
        id: TransportId,
        tick_tx: Option<NativeTickSender>,
        reconnect: Option<NativeReconnectFactory>,
    ) -> Self {
        Self {
            id,
            inner: Some(inner),
            init_error: None,
            tick_tx,
            connected: AtomicBool::new(false),
            disconnected_emitted: AtomicBool::new(false),
            handshake_headers: Vec::new(),
            reconnect,
        }
    }

    pub(crate) fn reconnect_factory(&self) -> Option<NativeReconnectFactory> {
        self.reconnect.clone()
    }

    async fn emit(&self, tick: Tick) {
        if let Some(tx) = &self.tick_tx {
            tx.send(tick).await.ok();
        }
    }

    async fn emit_disconnected_once(&self) {
        // 幂等去重：仅在「已连接且尚未发过 Disconnected」时发一次（内部可变，`&self`）。
        if self.connected.load(Ordering::Relaxed)
            && !self.disconnected_emitted.load(Ordering::Relaxed)
        {
            self.emit(Tick::Disconnected(self.id)).await;
            self.disconnected_emitted.store(true, Ordering::Relaxed);
            self.connected.store(false, Ordering::Relaxed);
        }
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
    /// 只需共享引用（与 Transport port `&self` 对齐，消除外层锁）。
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

#[async_trait::async_trait]
impl Transport for NativeTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        // 握手头灌进共享 header registry：SharedWsClient::connect 会把 registry 快照
        // 逐条注入 WS upgrade 请求（现网 Go cookieId=userId 建 session 依赖此）。
        if !self.handshake_headers.is_empty() {
            let registry = self.inner_mut()?.headers();
            registry
                .replace_headers(self.handshake_headers.iter().map(|(k, v)| (k, v)))
                .await?;
        }

        self.inner_mut()?.connect().await?;
        self.connected.store(true, Ordering::Relaxed);
        self.disconnected_emitted.store(false, Ordering::Relaxed);
        self.emit(Tick::Connected(self.id)).await;
        Ok(())
    }

    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        // 出站走 SharedWsClient::send（已修为 Message::Text，现网 Go Text→JSON 解码）。
        self.inner()?.send(frame).await
    }

    async fn recv(&self) -> Result<Option<Bytes>, PortError> {
        // 生产 native 路径（tick_tx=Some）入站走 Tick::Inbound，engine_loop 不调本方法；
        // 此方法仅服务单测手动驱动路径（tick_tx=None → SharedWsClient frame_rx 拉取）。
        let result = self.inner()?.recv().await?;
        if result.is_none() {
            self.emit_disconnected_once().await;
        }
        Ok(result)
    }

    async fn close(&self) -> Result<(), PortError> {
        let result = self.inner()?.close().await;
        self.emit_disconnected_once().await;
        result
    }
}

#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;
