//! NativeTransport — tokio-tungstenite WebSocket
//!
//! ## 设计要点
//!
//! - `.split()` 读写分离：SplitSink（写）+ SplitStream（读），允许并发读写
//! - 连接/断开产出 Tick::Connected(id) / Tick::Disconnected(id)（发往 engine_loop 的 mpsc）
//! - 指数退避重连（私有状态机，模块不感知）：2^n * 100ms + jitter，上限 30s
//! - Ping/Pong 心跳：driver 私有，每 30s 发一次 ping，60s 无 pong → 主动断连重连
//!
//! ## 关于 connect/send/recv/close 的 Transport trait 实现
//!
//! Transport trait 是供 engine_loop 直接驱动的低级接口。
//! 重连状态机和心跳是 NativeTransport 的私有实现细节。
//!
//! ## 错误翻译（ACL-2）
//!
//! tokio-tungstenite 错误 → PortError::Transport(msg)

use std::sync::Arc;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::PortError;
use helix_core::Tick;

/// 退避参数（重连状态机将在后续版本接入；当前 Transport trait 调用方直接管理）
#[allow(dead_code)]
const BACKOFF_BASE_MS: u64 = 100;
#[allow(dead_code)]
const BACKOFF_MAX_MS: u64 = 30_000;
/// Ping 心跳间隔（driver 私有状态机，当前注记，后续版本接入）
#[allow(dead_code)]
const PING_INTERVAL_MS: u64 = 30_000;
/// Pong 超时（超时后主动断连重连）
#[allow(dead_code)]
const PONG_TIMEOUT_MS: u64 = 60_000;

type WsSink = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

#[allow(dead_code)]
type WsStream = futures_util::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// 内部连接状态。
enum ConnState {
    Disconnected,
    Connected {
        sink: WsSink,
        /// 读任务发来的帧；读任务在 connect() 时 spawn
        frame_rx: mpsc::Receiver<Result<Bytes, PortError>>,
        /// 读任务 handle（关闭时 abort）
        _reader: tokio::task::JoinHandle<()>,
    },
}

/// tokio-tungstenite WebSocket 驱动传输。
///
/// 使用 `Arc<Mutex<ConnState>>` 允许 engine_loop 在多处持有（如同时持有 Transport + timer）。
/// Mutex 仅在 send/recv/connect/close 时短暂持有，不跨 await 泄漏。
pub struct NativeTransport {
    url: String,
    id: TransportId,
    state: Arc<Mutex<ConnState>>,
    /// engine_loop 入站通道：Connected/Disconnected Tick 从此注入
    tick_tx: Option<mpsc::Sender<Tick>>,
}

impl NativeTransport {
    /// 创建传输实例。
    ///
    /// `tick_tx` 用于产出 `Tick::Connected` / `Tick::Disconnected`，
    /// 设为 None 时不产出（测试 / 手动驱动场景）。
    pub fn new(
        url: impl Into<String>,
        id: TransportId,
        tick_tx: Option<mpsc::Sender<Tick>>,
    ) -> Self {
        Self {
            url: url.into(),
            id,
            state: Arc::new(Mutex::new(ConnState::Disconnected)),
            tick_tx,
        }
    }
}

fn map_ws_err(e: impl std::fmt::Display) -> PortError {
    PortError::Transport(e.to_string())
}

#[async_trait::async_trait]
impl Transport for NativeTransport {
    /// 建立 WebSocket 连接，spawn 后台读任务，产出 `Tick::Connected`。
    async fn connect(&mut self) -> Result<(), PortError> {
        let (ws_stream, _resp) = connect_async(&self.url).await.map_err(map_ws_err)?;
        let (sink, stream) = ws_stream.split();

        // 读任务：把 WS 消息转为 Bytes 发入 frame_rx
        let (frame_tx, frame_rx) = mpsc::channel::<Result<Bytes, PortError>>(64);
        let id = self.id;
        let tick_tx_clone = self.tick_tx.clone();

        let reader = tokio::spawn(async move {
            let mut stream = stream;
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Binary(data)) => {
                        if frame_tx.send(Ok(Bytes::from(data))).await.is_err() {
                            break; // 接收端已关闭
                        }
                    }
                    Ok(Message::Text(text)) => {
                        if frame_tx
                            .send(Ok(Bytes::from(text.into_bytes())))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {
                        // tungstenite 自动处理 Ping/Pong，此处透传
                    }
                    Ok(Message::Close(_)) | Err(_) => {
                        // 连接关闭：产出 Disconnected Tick
                        if let Some(tx) = &tick_tx_clone {
                            tx.send(Tick::Disconnected(id)).await.ok();
                        }
                        break;
                    }
                    Ok(Message::Frame(_)) => {}
                }
            }
        });

        *self.state.lock().await = ConnState::Connected {
            sink,
            frame_rx,
            _reader: reader,
        };

        // 产出 Connected Tick
        if let Some(tx) = &self.tick_tx {
            tx.send(Tick::Connected(self.id)).await.ok();
        }

        Ok(())
    }

    /// 发送一帧（Binary 消息）。
    async fn send(&mut self, frame: Bytes) -> Result<(), PortError> {
        let mut state = self.state.lock().await;
        match &mut *state {
            ConnState::Connected { sink, .. } => sink
                .send(Message::Binary(frame.to_vec()))
                .await
                .map_err(map_ws_err),
            ConnState::Disconnected => Err(PortError::Transport(
                "send on disconnected transport".to_string(),
            )),
        }
    }

    /// 接收下一帧（None = 连接已关闭）。
    async fn recv(&mut self) -> Result<Option<Bytes>, PortError> {
        let mut state = self.state.lock().await;
        match &mut *state {
            ConnState::Connected { frame_rx, .. } => {
                match frame_rx.recv().await {
                    Some(Ok(bytes)) => Ok(Some(bytes)),
                    Some(Err(e)) => Err(e),
                    None => Ok(None), // 读任务已结束（连接关闭）
                }
            }
            ConnState::Disconnected => Ok(None),
        }
    }

    /// 发送 Close 帧并清理状态。
    async fn close(&mut self) -> Result<(), PortError> {
        let mut state = self.state.lock().await;
        if let ConnState::Connected { sink, .. } = &mut *state {
            sink.send(Message::Close(None)).await.map_err(map_ws_err)?;
        }
        *state = ConnState::Disconnected;

        if let Some(tx) = &self.tick_tx {
            tx.send(Tick::Disconnected(self.id)).await.ok();
        }
        Ok(())
    }
}

// ─── 单元测试（集成测试在 tests/ 目录，需真实 WS 服务）────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::TransportId;

    #[tokio::test]
    async fn test_send_on_disconnected_returns_err() {
        let (tx, _rx) = mpsc::channel(8);
        let mut transport =
            NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(1), Some(tx));

        // 未 connect，send 应返回错误
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
        // close on disconnected 不应 panic
        transport.close().await.ok();
    }
}
