//! engine_loop — native 薄壳（pump 核已下沉 helix-driver-host::engine）。
//!
//! ## 下沉后的分工
//!
//! pump 核（`BoundedSpawner` / `OwnedEffect`+`consume_effects` / `dispatch_effects` /
//! `TimerRegistry` / `run_http_envelope` / 双通道 biased select + graceful drain）统一在
//! `helix-driver-host::engine`（泛型 over Storage/HttpRequester/EventSink/Transport/Clock）。
//! native 这层只保留：
//! - 具体 ports 装配（`NativeStorage` / `NativeHttp` / `NativeEventSink` / `NativeClock`）
//! - 与现网兼容的公开 API 签名（[`EngineConfig`] / [`TransportTable`] / [`run_engine_loop`]）
//! - tick 源接线（`SharedWsClient::with_inbound_tick`，在 `transport.rs`）
//!
//! NativeEventSink（broadcast egress）特化留 native；（下一轮）FFI 传批处理 vtable sink，
//! 共用同一 host 泛型壳，只差 egress + tick 源。
//!
//! ## 五不变量真源
//!
//! 五不变量（回灌必达 / 主循环不等 I/O / 同 key 写序 / 回灌优先 / graceful drain）
//! 的实现与文档真源现在在 `helix-driver-host::engine` 头注 + 本 crate AGENTS.md。

use std::sync::Arc;

use helix_driver_host::engine::{self, EngineDeps, TransportLifecycleEvent, TransportTraceEvent};
use helix_driver_host::{
    spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink, SharedFileUploader, TraceHooks,
};
// rows↔bytes 编解码：host 单一真源，native re-export（helix-im 反序列化端对齐 host codec）。
pub use helix_driver_host::{rows_from_reply_bytes, rows_to_reply_bytes};

use helix_core::effect::TransportId;
use helix_core::{ExecutionShell, Tick};
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

use crate::clock::NativeClock;
use crate::event_sink::NativeEventSink;
use crate::http::NativeHttp;
use crate::storage::NativeStorage;
use crate::transport::NativeTransport;

/// TransportId → 实际 WS 连接 的路由表（native 特化：值为 `NativeTransport`）。
///
/// host 装配阶段填入（`host.add_transport(...)` → `TransportId`）；host 泵的 `Effect::Send`
/// 分支按 id 查表调 `NativeTransport::send`（`&self`）。多连接形态从一开始做成 `HashMap`。
/// `Arc<_>`（无外层 `Mutex`）：send/recv/close 已收紧 `&self`（host `SharedWsClient` 内部 state 锁
/// 串行化），表里只需共享句柄；connect 在装配端独占 `&mut` 持有时完成（见 host engine 头注）。
pub type TransportTable = HashMap<TransportId, Arc<NativeTransport>>;

/// 事件泵配置（native 具体 ports 聚合）。
///
/// 装配后由 [`run_engine_loop`] / [`run_engine_loop_with_transports`] 转成 host 的
/// [`EngineDeps`]（注入 `NativeClock`），调泛型壳 `helix_driver_host::engine::run_engine_loop`。
pub struct EngineConfig {
    pub storage: NativeStorage,
    pub http: NativeHttp,
    pub uploader: SharedFileUploader,
    pub event_sink: NativeEventSink,
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。
    ///
    /// 冷启动时 core 一步可吐 N 条 `Effect::Http`；若全部齐发会瞬间打满底层连接
    /// （reqwest pool **不**限制新连接并发）。此字段 = Http/HttpFire BoundedSpawner 的常驻
    /// worker 数：第 N+1 条请求在有界队列里等 worker，不并发齐发。host（ambient API 合法落点）
    /// 读 env 注入；建议默认 8。`0` 会被 `.max(1)` 兜底为 1。
    pub max_http_inflight: usize,
}

impl EngineConfig {
    /// 转成 host 泛型壳所需的 [`EngineDeps`]（注入 `NativeClock`）。
    fn into_deps(
        self,
        transport_lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
        transport_trace_tx: Option<mpsc::UnboundedSender<TransportTraceEvent>>,
    ) -> EngineDeps<NativeStorage, NativeHttp, SharedFileUploader, NativeEventSink, NativeClock>
    {
        EngineDeps {
            storage: Arc::new(self.storage),
            http: Arc::new(self.http),
            uploader: Arc::new(self.uploader),
            event_sink: Arc::new(self.event_sink),
            clock: NativeClock,
            trace: TraceHooks::noop(),
            max_http_inflight: self.max_http_inflight,
            transport_lifecycle_tx,
            transport_trace_tx,
        }
    }
}

/// 运行 ExecutionShell 的 tokio 事件泵（native 入口，无 transport 路由）。
///
/// 无 transport 的兼容入口：Send 表为空 → `Effect::Send` 落 warn（回退既有行为）。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
/// let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
/// let config = EngineConfig {
///     storage,
///     http,
///     uploader: helix_driver_host::SharedFileUploader::default(),
///     event_sink,
///     max_http_inflight: 8,
/// };
/// tokio::spawn(run_engine_loop(shell, tick_rx, tick_tx.clone(), config, shutdown_rx));
/// ```
///
/// ## Shutdown 语义
///
/// `shutdown_tx.send(())` 后进入 graceful drain，返回 = 全部已接收 Persist + 必达 Http 已落地。
pub async fn run_engine_loop(
    shell: ExecutionShell,
    tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    shutdown_rx: oneshot::Receiver<()>,
) {
    run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx,
        config,
        shutdown_rx,
        TransportTable::new(),
    )
    .await
}

/// 带 transport 路由表的事件泵入口（A1）。
///
/// 与 [`run_engine_loop`] 唯一区别：`transports` 把 `TransportId → NativeTransport`
/// 接进泵，使 `Effect::Send` 真正发到 WS。host 装配时 `connect()` 各 transport 后填表。
///
/// 装配 native ports → host 泛型壳 `engine::run_engine_loop`（单一 pump 核）。
pub async fn run_engine_loop_with_transports(
    shell: ExecutionShell,
    tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable,
) {
    let NativeReconnectRuntime {
        transport_rx,
        lifecycle_tx,
        trace_tx,
        tasks,
    } = start_native_reconnect(&transports);
    engine::run_engine_loop(
        shell,
        tick_rx,
        tick_tx,
        config.into_deps(lifecycle_tx, trace_tx),
        shutdown_rx,
        transports,
        transport_rx,
    )
    .await;
    for task in tasks {
        task.abort();
    }
}

struct NativeReconnectRuntime {
    transport_rx: mpsc::UnboundedReceiver<(TransportId, Arc<NativeTransport>)>,
    lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
    trace_tx: Option<mpsc::UnboundedSender<TransportTraceEvent>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

fn start_native_reconnect(transports: &TransportTable) -> NativeReconnectRuntime {
    let (register_tx, register_rx) = mpsc::unbounded_channel();
    let mut lifecycle_routes = HashMap::new();
    let (trace_tx, trace_rx) = mpsc::unbounded_channel();
    let mut tasks = Vec::new();

    for (&transport_id, transport) in transports {
        let Some(factory) = transport.reconnect_factory() else {
            continue;
        };
        let (route_tx, route_rx) = mpsc::unbounded_channel();
        lifecycle_routes.insert(transport_id, route_tx);
        let register_tx = register_tx.clone();
        let trace_sink = ReconnectTraceSink::new(Some(trace_tx.clone()));
        tasks.push(spawn_reconnect_supervisor(
            transport_id,
            ReconnectPolicy::default(),
            route_rx,
            register_tx,
            trace_sink,
            move || {
                let factory = factory.clone();
                async move {
                    let mut transport = factory.build()?;
                    helix_core::ports::Transport::connect(&mut transport).await?;
                    Ok(transport)
                }
            },
        ));
    }
    drop(register_tx);

    if lifecycle_routes.is_empty() {
        return NativeReconnectRuntime {
            transport_rx: register_rx,
            lifecycle_tx: None,
            trace_tx: None,
            tasks,
        };
    }

    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    tasks.push(tokio::spawn(async move {
        while let Some(event) = lifecycle_rx.recv().await {
            let TransportLifecycleEvent::Disconnected { transport_id, .. } = event;
            if let Some(route) = lifecycle_routes.get(&transport_id) {
                route.send(event).ok();
            }
        }
    }));
    tasks.push(spawn_transport_trace_logger(trace_rx));

    NativeReconnectRuntime {
        transport_rx: register_rx,
        lifecycle_tx: Some(lifecycle_tx),
        trace_tx: Some(trace_tx),
        tasks,
    }
}

fn spawn_transport_trace_logger(
    mut rx: mpsc::UnboundedReceiver<TransportTraceEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            tracing::info!(
                target: "helix_ws_lifecycle",
                transport_id = event.transport_id.raw(),
                action = event.action,
                attempt = ?event.attempt,
                delay_ms = ?event.delay_ms,
                next_delay_ms = ?event.next_delay_ms,
                reason = ?event.reason,
                error_class = ?event.error_class,
                "Helix WebSocket lifecycle"
            );
        }
    })
}

// ─── 单元测试（native 装配层 smoke；pump 核原语测试已随下沉到 host）──────────────

#[cfg(test)]
mod tests {
    use helix_driver_host::{rows_from_reply_bytes, rows_to_reply_bytes};

    use helix_core::effect::SqlValue;

    /// rows↔bytes 编解码经 native re-export 仍可达（host 单一真源，native 转出）。
    #[test]
    fn test_rows_roundtrip_via_reexport() {
        let rows = vec![vec![
            ("id".to_string(), SqlValue::Integer(1)),
            ("name".to_string(), SqlValue::Text("alice".to_string())),
            ("score".to_string(), SqlValue::Real(9.5)),
            ("extra".to_string(), SqlValue::Null),
        ]];
        let bytes = rows_to_reply_bytes(&rows);
        let restored = rows_from_reply_bytes(&bytes).expect("roundtrip");
        assert_eq!(restored.len(), 1);
        let row = &restored[0];
        assert!(matches!(
            row.iter().find(|(k, _)| k == "id").map(|(_, v)| v),
            Some(SqlValue::Integer(1))
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "name").map(|(_, v)| v),
            Some(SqlValue::Text(s)) if s == "alice"
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "extra").map(|(_, v)| v),
            Some(SqlValue::Null)
        ));
    }
}
