//! Shared HTTP and WebSocket clients for PC Tauri and Flutter FFI.
//!
//! Login/auth remains outside Helix. Platform shells provide config and update
//! headers; these clients only inject the latest header snapshot into requests.

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::tick::InboundBytes;
use helix_core::{PortError, Tick};
use reqwest::header::USER_AGENT;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

use crate::metrics::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

// URL/header 工具裂分到 sibling（按职责，structure-gate §1）。`use *` 复用 6 个 pub(super) helper。
#[path = "network_util.rs"]
mod network_util;
use network_util::{header_value, headers_to_strings, map_ws_err, validate_url};

mod config;
pub use config::{HostHeaderRegistry, HostNetworkConfig};

mod http;
pub use http::{PreparedHttpRequest, SharedHttpClient};

const WS_FRAME_BUFFER: usize = 64;

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

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
    disconnect_notified: Arc<AtomicBool>,
    metrics: Arc<dyn AsyncMetricSink>,
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
            disconnect_notified: Arc::new(AtomicBool::new(false)),
            metrics: Arc::new(NoopMetricSink),
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

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
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
        let handshake_headers = headers_to_strings(request.headers());
        crate::network_debug::dump_ws_handshake(&self.config.ws_url, &handshake_headers);

        let (ws, _resp) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(map_ws_err)?;
        let (sink, mut stream) = ws.split();
        let (frame_tx, frame_rx) = mpsc::channel::<Result<Bytes, PortError>>(WS_FRAME_BUFFER);

        self.disconnect_notified.store(false, Ordering::Release);
        let inbound = self.inbound.clone();
        let disconnect_notified = Arc::clone(&self.disconnect_notified);
        let metrics = Arc::clone(&self.metrics);

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

                if let Ok(bytes) = &frame {
                    crate::network_debug::dump_ws_inbound(bytes);
                    if metrics.is_enabled() {
                        let _ = metrics.try_record(MetricEvent::counter(
                            MetricId::OperationsTotal,
                            1.0,
                            MetricLabels::one(LabelKey::Stage, "ws")
                                .with(LabelKey::Protocol, "ws")
                                .with(LabelKey::Direction, "inbound")
                                .with(LabelKey::Status, "ok"),
                        ));
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
            notify_disconnected(&inbound, &disconnect_notified);
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
        crate::network_debug::dump_ws_outbound(&frame);
        let mut state = self.state.lock().await;
        let result = match &mut *state {
            WsState::Connected { sink, .. } => {
                let text = String::from_utf8(frame.to_vec())
                    .map_err(|e| PortError::Transport(format!("non-utf8 ws frame: {e}")))?;
                sink.send(Message::Text(text)).await.map_err(map_ws_err)
            }
            WsState::Disconnected => Err(PortError::Transport(
                "send on disconnected transport".to_string(),
            )),
        };
        if result.is_err() {
            let previous = std::mem::replace(&mut *state, WsState::Disconnected);
            if let WsState::Connected { reader, .. } = previous {
                reader.abort();
            }
        }
        drop(state);
        if result.is_err() {
            notify_disconnected(&self.inbound, &self.disconnect_notified);
        }
        if self.metrics.is_enabled() {
            let status = if result.is_ok() { "ok" } else { "error" };
            let labels = MetricLabels::one(LabelKey::Stage, "ws")
                .with(LabelKey::Protocol, "ws")
                .with(LabelKey::Direction, "outbound")
                .with(LabelKey::Status, status);
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::OperationsTotal,
                1.0,
                labels,
            ));
            if result.is_err() {
                let _ = self.metrics.try_record(MetricEvent::counter(
                    MetricId::ErrorsTotal,
                    1.0,
                    labels.with(LabelKey::ErrorKind, "ws_send_failed"),
                ));
            }
        }
        result
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
pub(super) fn notify_disconnected(
    inbound: &Option<(TransportId, mpsc::Sender<Tick>)>,
    notified: &AtomicBool,
) {
    if notified
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    if let Some((id, tick_tx)) = inbound {
        let tick = Tick::Disconnected(*id);
        if let Err(mpsc::error::TrySendError::Full(tick)) = tick_tx.try_send(tick) {
            let tick_tx = tick_tx.clone();
            tokio::spawn(async move {
                tick_tx.send(tick).await.ok();
            });
        }
    }
}
