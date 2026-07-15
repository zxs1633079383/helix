//! Native WebSocket thin shell over the shared host client.
//!
//! WS 协议执行（连接/收发/Text 出站/入站读任务）统一在 `helix-driver-host` 的
//! `SharedWsClient`（ADR-007 双端共享）。native 只保留预填表激活接缝与现网兼容构造器：
//! - **握手头**（`with_handshake_headers`）：现网 Go 在 WS 升级阶段按 `cookieId=userId` 建 session，
//!   session 空则不发 hello、~5s 超时关连接，故身份头必须随 upgrade 请求带（灌进共享 header registry）。
//! - **入站推送**（`SharedWsClient::with_inbound_tick`）：native engine_loop 从不调 `recv()`，
//!   入站帧全靠读任务作 `Tick::Inbound` 直发泵（A2 不变量）。

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::FrameSender;
use helix_core::PortError;
use helix_core::Tick;
use helix_driver_host::{HostNetworkConfig, SharedWsClient, WsConnectionActivation};
use tokio::sync::mpsc;

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
        let inner = SharedWsClient::new(config)?;
        Ok(Self::from_shared(inner, id, tick_tx))
    }

    pub fn from_shared(
        mut inner: SharedWsClient,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        // 生产 native 路径把 Connected/Inbound/Disconnected 全部交给 shared client 的
        // 每连接生命周期；pull 单测不注入 tick，仍从 frame_rx 拉取。
        if let Some(tx) = tick_tx {
            inner = inner.with_inbound_tick(id, tx);
        }
        Self {
            inner: Some(inner),
            init_error: None,
            handshake_headers: Vec::new(),
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
        }

        self.inner_mut()?.connect().await
    }

    async fn send_frame(&self, frame: Bytes) -> Result<(), PortError> {
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
