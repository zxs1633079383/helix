//! engine_loop — native 薄壳（pump 核已下沉 helix-driver-host::engine）。
//!
//! ## 下沉后的分工
//!
//! pump 核（`BoundedSpawner` / `OwnedEffect`+`consume_effects` / `dispatch_effects` /
//! `TimerRegistry` / `run_http_envelope` / 双通道 biased select + graceful drain）统一在
//! `helix-driver-host::engine`（泛型 over Storage/HttpRequester/EventSink/FrameSender/Clock）。
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

use helix_driver_host::engine::{self, EngineDeps, TransportLifecycleEvent, TransportTraceSink};
use helix_driver_host::{AsyncMetricSink, NoopMetricSink, SharedFileUploader, TraceHooks};
use helix_driver_host::{TickIngressReceiver, TickIngressSender};
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

mod reconnect;
use reconnect::{start_native_reconnect, NativeReconnectRuntime};

/// TransportId → 实际 WS 连接 的路由表（native 特化：值为 `NativeTransport`）。
///
/// host 装配阶段填入（`host.add_transport(...)` → `TransportId`）；host 泵的 `Effect::Send`
/// 分支按 id 查表调 `NativeTransport::send`（`&self`）。多连接形态从一开始做成 `HashMap`。
/// `Arc<_>`（无外层 `Mutex`）：FrameSender::send 只需 `&self`（host `SharedWsClient` 内部 state
/// 锁串行化），表里只需共享句柄；connect 在装配端独占 `&mut` 持有时完成（见 host engine 头注）。
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
    /// Engine 热路径指标 sink；composition root 应与 HTTP/WS/Storage/Event 共用同一 Arc。
    pub metrics: Arc<dyn AsyncMetricSink>,
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。
    ///
    /// 冷启动时 core 一步可吐 N 条 `Effect::Http`；若全部齐发会瞬间打满底层连接
    /// （reqwest pool **不**限制新连接并发）。此字段 = Http/HttpFire BoundedSpawner 的常驻
    /// worker 数：第 N+1 条请求在有界队列里等 worker，不并发齐发。host（ambient API 合法落点）
    /// 读 env 注入；建议默认 8。`0` 会被 `.max(1)` 兜底为 1。
    pub max_http_inflight: usize,
}

impl EngineConfig {
    pub fn new(
        storage: NativeStorage,
        http: NativeHttp,
        uploader: SharedFileUploader,
        event_sink: NativeEventSink,
        max_http_inflight: usize,
    ) -> Self {
        Self {
            storage,
            http,
            uploader,
            event_sink,
            metrics: Arc::new(NoopMetricSink),
            max_http_inflight,
        }
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }

    /// 转成 host 泛型壳所需的 [`EngineDeps`]（注入 `NativeClock`）。
    fn into_deps(
        self,
        trace: TraceHooks,
        transport_lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
        transport_trace_tx: Option<TransportTraceSink>,
    ) -> EngineDeps<NativeStorage, NativeHttp, SharedFileUploader, NativeEventSink, NativeClock>
    {
        EngineDeps {
            storage: Arc::new(self.storage),
            http: Arc::new(self.http),
            uploader: Arc::new(self.uploader),
            event_sink: Arc::new(self.event_sink),
            clock: NativeClock,
            trace,
            metrics: self.metrics,
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
///     metrics: std::sync::Arc::new(helix_driver_host::NoopMetricSink),
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

/// 生产指标入口：保持单一有界队列，在元素内携带真实入队时间戳。
pub async fn run_engine_loop_stamped(
    shell: ExecutionShell,
    tick_rx: TickIngressReceiver,
    tick_tx: TickIngressSender,
    config: EngineConfig,
    shutdown_rx: oneshot::Receiver<()>,
) {
    run_engine_loop_with_transports_and_trace_stamped(
        shell,
        tick_rx,
        tick_tx,
        config,
        TraceHooks::noop(),
        shutdown_rx,
        TransportTable::new(),
    )
    .await;
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
    run_engine_loop_with_transports_and_trace(
        shell,
        tick_rx,
        tick_tx,
        config,
        TraceHooks::noop(),
        shutdown_rx,
        transports,
    )
    .await;
}

/// 显式注入 trace hooks 的 native composition-root 入口；旧入口保持真 no-op。
pub async fn run_engine_loop_with_transports_and_trace(
    shell: ExecutionShell,
    tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    trace: TraceHooks,
    shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable,
) {
    let NativeReconnectRuntime {
        transport_rx,
        lifecycle_tx,
        trace_sink,
        trace_stats,
        tasks,
        shutdown_tx: reconnect_shutdown_tx,
    } = start_native_reconnect(&transports, &config.event_sink);
    engine::run_engine_loop(
        shell,
        tick_rx,
        tick_tx,
        config.into_deps(trace, lifecycle_tx, trace_sink),
        shutdown_rx,
        transports,
        transport_rx,
    )
    .await;
    if let Some(stats) = trace_stats {
        let dropped = stats.dropped_count();
        if dropped > 0 {
            tracing::warn!(dropped, "native transport trace 有界队列发生丢弃");
        }
    }
    reconnect_shutdown_tx.send_replace(true);
    for task in tasks {
        task.abort();
    }
}

pub async fn run_engine_loop_with_transports_and_trace_stamped(
    shell: ExecutionShell,
    tick_rx: TickIngressReceiver,
    tick_tx: TickIngressSender,
    config: EngineConfig,
    trace: TraceHooks,
    shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable,
) {
    let NativeReconnectRuntime {
        transport_rx,
        lifecycle_tx,
        trace_sink,
        trace_stats,
        tasks,
        shutdown_tx: reconnect_shutdown_tx,
    } = start_native_reconnect(&transports, &config.event_sink);
    engine::run_engine_loop_stamped(
        shell,
        tick_rx,
        tick_tx,
        config.into_deps(trace, lifecycle_tx, trace_sink),
        shutdown_rx,
        transports,
        transport_rx,
    )
    .await;
    if let Some(stats) = trace_stats {
        let dropped = stats.dropped_count();
        if dropped > 0 {
            tracing::warn!(dropped, "native transport trace 有界队列发生丢弃");
        }
    }
    reconnect_shutdown_tx.send_replace(true);
    for task in tasks {
        task.abort();
    }
}

#[cfg(test)]
#[path = "engine_loop_tests.rs"]
mod tests;
