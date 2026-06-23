//! Shared HTTP and WebSocket clients for PC Tauri and Flutter FFI.
//!
//! Login/auth remains outside Helix. Platform shells provide config and update
//! headers; these clients only inject the latest header snapshot into requests.

use parking_lot::RwLock; // 无 poison：panic 时不静默丢头（HTTP-4，对齐现网 cses-client）
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use helix_core::effect::{HttpRequest, HttpResponse, TransportId};
use helix_core::ports::{HttpRequester, Transport};
use helix_core::tick::InboundBytes;
use helix_core::{PortError, Tick};
use reqwest::header::{HeaderMap, USER_AGENT};
use reqwest::{Client, Method};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

// URL/header 工具裂分到 sibling（按职责，structure-gate §1）。`use *` 复用 6 个 pub(super) helper。
#[path = "network_util.rs"]
mod network_util;
use network_util::{
    header_name, header_value, headers_to_strings, map_ws_err, resolve_http_url, validate_url,
};

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const WS_FRAME_BUFFER: usize = 64;

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

/// Platform-provided network initialization material.
#[derive(Debug, Clone)]
pub struct HostNetworkConfig {
    pub api_base_url: String,
    /// 第二网关 base URL（spec06 方案 A）：= `env.restHost`（无 `/api/cses`）；vote/score 走它。空串=未注入。
    pub default_api_base_url: String,
    pub ws_url: String,
    pub timeout: Duration,
    pub user_agent: Option<String>,
}

impl HostNetworkConfig {
    pub fn new(api_base_url: impl Into<String>, ws_url: impl Into<String>) -> Self {
        Self {
            api_base_url: api_base_url.into(),
            default_api_base_url: String::new(),
            ws_url: ws_url.into(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            user_agent: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = Some(user_agent.into());
        self
    }
}

/// Shared mutable header registry used by HTTP requests and WS handshakes.
#[derive(Clone, Default)]
pub struct HostHeaderRegistry {
    inner: Arc<RwLock<HeaderMap>>,
}

impl HostHeaderRegistry {
    pub async fn set_header(&self, name: &str, value: &str) -> Result<(), PortError> {
        self.set_header_sync(name, value)
    }

    pub fn set_header_sync(&self, name: &str, value: &str) -> Result<(), PortError> {
        let name = header_name(name)?;
        let value = header_value(name.as_str(), value)?;
        self.inner.write().insert(name, value);
        Ok(())
    }

    pub async fn remove_header(&self, name: &str) -> Result<(), PortError> {
        self.remove_header_sync(name)
    }

    pub fn remove_header_sync(&self, name: &str) -> Result<(), PortError> {
        let name = header_name(name)?;
        self.inner.write().remove(name);
        Ok(())
    }

    pub async fn replace_headers<I, K, V>(&self, headers: I) -> Result<(), PortError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut next = HeaderMap::new();
        for (name, value) in headers {
            let name = header_name(name.as_ref())?;
            let value = header_value(name.as_str(), value.as_ref())?;
            next.insert(name, value);
        }
        *self.inner.write() = next;
        Ok(())
    }

    pub async fn snapshot(&self) -> HeaderMap {
        self.inner.read().clone()
    }
}

/// Request shape after shared config/header resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Bytes>,
}

/// Shared reqwest-backed HTTP port implementation.
#[derive(Clone)]
pub struct SharedHttpClient {
    client: Client,
    config: HostNetworkConfig,
    headers: HostHeaderRegistry,
}

impl SharedHttpClient {
    pub fn new(config: HostNetworkConfig) -> Result<Self, PortError> {
        Self::with_registry(config, HostHeaderRegistry::default())
    }

    pub fn with_registry(
        config: HostNetworkConfig,
        headers: HostHeaderRegistry,
    ) -> Result<Self, PortError> {
        validate_url(&config.api_base_url, "api_base_url")?;
        // 第二网关 base 仅在已注入（非空）时校验（空串 = vote/score 暂不用）。
        if !config.default_api_base_url.is_empty() {
            validate_url(&config.default_api_base_url, "default_api_base_url")?;
        }
        let mut builder = Client::builder().timeout(config.timeout);
        if let Some(user_agent) = &config.user_agent {
            builder = builder.user_agent(user_agent);
        }
        let client = builder
            .build()
            .map_err(|e| PortError::Http(format!("reqwest client build error: {e}")))?;
        Ok(Self {
            client,
            config,
            headers,
        })
    }

    pub fn headers(&self) -> HostHeaderRegistry {
        self.headers.clone()
    }

    pub async fn prepare_for_test(
        &self,
        req: HttpRequest,
    ) -> Result<PreparedHttpRequest, PortError> {
        self.prepare(req).await
    }

    async fn prepare(&self, req: HttpRequest) -> Result<PreparedHttpRequest, PortError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid method '{}': {e}", req.method)))?;
        let url = resolve_http_url(&self.config.api_base_url, &req.url)?;

        let mut headers = self.headers.snapshot().await;
        if let Some(user_agent) = &self.config.user_agent {
            headers.insert(USER_AGENT, header_value("user-agent", user_agent)?);
        }
        for (name, value) in &req.headers {
            let name = header_name(name)?;
            let value = header_value(name.as_str(), value)?;
            headers.insert(name, value);
        }

        Ok(PreparedHttpRequest {
            method: method.as_str().to_string(),
            url,
            headers: headers_to_strings(&headers),
            body: req.body,
        })
    }

    fn classify_error(e: &reqwest::Error) -> PortError {
        if e.is_timeout() {
            PortError::Transport(format!("timeout: {e}"))
        } else if e.is_connect() || e.is_request() {
            PortError::Transport(format!("network: {e}"))
        } else {
            PortError::Http(e.to_string())
        }
    }
}

#[async_trait::async_trait]
impl HttpRequester for SharedHttpClient {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let prepared = self.prepare(req).await?;
        let method = Method::from_bytes(prepared.method.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid method '{}': {e}", prepared.method)))?;
        let mut builder = self.client.request(method, &prepared.url);

        for (name, value) in &prepared.headers {
            builder = builder.header(name, value);
        }
        if let Some(body) = prepared.body {
            builder = builder.body(body);
        }

        let resp = builder.send().await.map_err(|e| Self::classify_error(&e))?;
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or("<non-utf8>").to_string(),
                )
            })
            .collect();
        let body = resp.bytes().await.map_err(|e| Self::classify_error(&e))?;

        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

enum WsState {
    Disconnected,
    Connected {
        sink: WsSink,
        frame_rx: mpsc::Receiver<Result<Bytes, PortError>>,
        reader: tokio::task::JoinHandle<()>,
    },
}

/// Shared tokio-tungstenite-backed WebSocket port implementation.
///
/// 入站读任务有两条路径（按是否注入 `inbound`）：
/// - **push（native engine_loop）**：注入 `(id, tick_tx)` → 读任务把数据帧作 `Tick::Inbound`
///   直接喂泵，连接关闭/出错作 `Tick::Disconnected`。native engine_loop 从不调 `recv()`，
///   入站全靠此通道（A2，HX 入站不变量）。读任务独立持 stream 半边，不碰 send 锁。
/// - **pull（FFI / 单测）**：无 `inbound` → 读任务塞内部 `frame_rx`，由 `Transport::recv()` 拉取。
pub struct SharedWsClient {
    config: HostNetworkConfig,
    headers: HostHeaderRegistry,
    state: Arc<Mutex<WsState>>,
    /// 可选入站推送通道：Some 时读任务发 `Tick::Inbound`/`Disconnected`（native 路径）。
    inbound: Option<(TransportId, mpsc::Sender<Tick>)>,
}

impl SharedWsClient {
    pub fn new(config: HostNetworkConfig) -> Result<Self, PortError> {
        Self::with_registry(config, HostHeaderRegistry::default())
    }

    pub fn with_registry(
        config: HostNetworkConfig,
        headers: HostHeaderRegistry,
    ) -> Result<Self, PortError> {
        validate_url(&config.ws_url, "ws_url")?;
        Ok(Self {
            config,
            headers,
            state: Arc::new(Mutex::new(WsState::Disconnected)),
            inbound: None,
        })
    }

    pub fn headers(&self) -> HostHeaderRegistry {
        self.headers.clone()
    }

    /// 注入入站推送通道（native engine_loop 路径，A2）。
    ///
    /// 设置后，connect 的读任务把数据帧作 `Tick::Inbound` 直发 `tick_tx`、
    /// 连接关闭/出错作 `Tick::Disconnected(id)`，而非塞内部 `frame_rx`。
    pub fn with_inbound_tick(mut self, id: TransportId, tick_tx: mpsc::Sender<Tick>) -> Self {
        self.inbound = Some((id, tick_tx));
        self
    }
}

#[async_trait::async_trait]
impl Transport for SharedWsClient {
    async fn connect(&mut self) -> Result<(), PortError> {
        let mut request = self
            .config
            .ws_url
            .as_str()
            .into_client_request()
            .map_err(|e| PortError::Transport(format!("invalid ws request: {e}")))?;
        for (name, value) in self.headers.snapshot().await.iter() {
            request.headers_mut().insert(name.clone(), value.clone());
        }
        if let Some(user_agent) = &self.config.user_agent {
            request
                .headers_mut()
                .insert(USER_AGENT, header_value("user-agent", user_agent)?);
        }

        let (ws, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(map_ws_err)?;
        let (sink, mut stream) = ws.split();
        let (frame_tx, frame_rx) = mpsc::channel::<Result<Bytes, PortError>>(WS_FRAME_BUFFER);

        // 联调工具：HELIX_DUMP_FRAMES 置位时打印每条入站原始帧（抓真 Go wire 做 golden fixtures）。
        let dump_frames = std::env::var("HELIX_DUMP_FRAMES").is_ok();
        let inbound = self.inbound.clone();

        let reader = tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                // 归一化数据帧字节（Binary/Text 统一 Bytes）；关闭/出错单独处理。
                let frame = match msg {
                    Ok(Message::Binary(data)) => Ok(Bytes::from(data)),
                    Ok(Message::Text(text)) => Ok(Bytes::from(text.into_bytes())),
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => continue,
                    Err(e) => Err(map_ws_err(e)),
                };

                if dump_frames {
                    if let Ok(bytes) = &frame {
                        tracing::info!(
                            target: "helix_ws_frames",
                            len = bytes.len(),
                            frame = %String::from_utf8_lossy(bytes),
                            "RAW WS inbound"
                        );
                    }
                }

                match &inbound {
                    // push 路径（native engine_loop）：数据帧 → Tick::Inbound 直发泵。
                    // 读任务独立持 stream 半边、不碰 send 锁，故不与 send 抢锁（A2）。
                    Some((_, tick_tx)) => match frame {
                        Ok(bytes) => {
                            if tick_tx
                                .send(Tick::Inbound(InboundBytes(bytes)))
                                .await
                                .is_err()
                            {
                                break; // 泵已关闭
                            }
                        }
                        // 入站读出错（如非法帧）：视为连接异常，下面 loop 退出补 Disconnected。
                        Err(_) => break,
                    },
                    // pull 路径（FFI / 单测）：塞 frame_rx 供 recv 拉取。
                    None => {
                        if frame_tx.send(frame).await.is_err() {
                            break; // 接收端已关闭
                        }
                    }
                }
            }

            // push 路径在读任务结束（Close/出错/泵关）时补发 Disconnected，
            // 与 native 旧实现等价（pull 路径靠 recv 返回 None 触发 Disconnected）。
            if let Some((id, tick_tx)) = &inbound {
                tick_tx.send(Tick::Disconnected(*id)).await.ok();
            }
        });

        *self.state.lock().await = WsState::Connected {
            sink,
            frame_rx,
            reader,
        };
        Ok(())
    }

    /// 发送一帧（Text/JSON 消息）。
    ///
    /// 现网 Go 按 WS 帧类型选 codec：**Text→JSON 解码，Binary→msgpack 解码**。
    /// helix 出站帧是 JSON（hello/ping/业务），必须发 **Text**——发 Binary 则 Go 用 msgpack
    /// 解 `{`(0x7b) 失败：`msgpack: unexpected code=7b decoding map length` → 关连接
    /// （真 Go dev-loop 三测实测定位的 ~8s 断连根因，c823701）。双端（PC+Flutter）共享此修。
    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        let mut state = self.state.lock().await;
        match &mut *state {
            WsState::Connected { sink, .. } => {
                let text = String::from_utf8(frame.to_vec())
                    .map_err(|e| PortError::Transport(format!("non-utf8 ws frame: {e}")))?;
                sink.send(Message::Text(text)).await.map_err(map_ws_err)
            }
            WsState::Disconnected => Err(PortError::Transport(
                "send on disconnected transport".to_string(),
            )),
        }
    }

    async fn recv(&self) -> Result<Option<Bytes>, PortError> {
        let mut state = self.state.lock().await;
        match &mut *state {
            WsState::Connected { frame_rx, .. } => match frame_rx.recv().await {
                Some(Ok(bytes)) => Ok(Some(bytes)),
                Some(Err(e)) => Err(e),
                None => Ok(None),
            },
            WsState::Disconnected => Ok(None),
        }
    }

    async fn close(&self) -> Result<(), PortError> {
        let mut state = self.state.lock().await;
        if let WsState::Connected { sink, reader, .. } = &mut *state {
            sink.send(Message::Close(None)).await.map_err(map_ws_err)?;
            reader.abort();
        }
        *state = WsState::Disconnected;
        Ok(())
    }
}
