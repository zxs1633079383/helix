use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::metrics::AsyncMetricSink;
use crate::trace::TraceHooks;
use helix_core::effect::TransportId;

/// `TransportId → 实际连接` 路由表（泛型 over Transport 实现）。
///
/// host 装配阶段填入（或经 `transport_rx` 注册通道在连接成功后投递入表）；`Effect::Send`
/// 分支按 id 查表调 `Transport::send`（`&self`）。表内 `Arc<Tr>` 无外层锁，连接任务只在
/// 成功后把独占句柄注册进表，避免连接期锁跨 `await`。
pub type TransportTable<Tr> = HashMap<TransportId, Arc<Tr>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportLifecycleEvent {
    Disconnected {
        transport_id: TransportId,
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportTraceEvent {
    pub transport_id: TransportId,
    pub name: &'static str,
    pub action: &'static str,
    pub attempt: Option<u32>,
    pub delay_ms: Option<u64>,
    pub next_delay_ms: Option<u64>,
    pub reason: Option<&'static str>,
    pub error_class: Option<String>,
}

/// Transport 生命周期遥测的固定有界队列容量。
///
/// 这是可丢诊断面，不参与 transport/control 正确性；过载时丢最新事件并累计计数，
/// 绝不把 exporter 背压传进 engine 热路径。
pub const TRANSPORT_TRACE_QUEUE_CAPACITY: usize = 256;

#[derive(Clone, Debug, Default)]
pub struct TransportTraceStats {
    dropped: Arc<AtomicU64>,
}

impl TransportTraceStats {
    pub fn dropped_count(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// 有界、同步非阻塞的 transport trace 出口。
#[derive(Clone, Debug)]
pub struct TransportTraceSink {
    tx: mpsc::Sender<TransportTraceEvent>,
    stats: TransportTraceStats,
}

impl TransportTraceSink {
    pub fn channel() -> (Self, mpsc::Receiver<TransportTraceEvent>) {
        let (tx, rx) = mpsc::channel(TRANSPORT_TRACE_QUEUE_CAPACITY);
        let stats = TransportTraceStats::default();
        (Self { tx, stats }, rx)
    }

    /// 队满或接收端关闭时立即返回，丢本条并累计，不 await、不阻塞。
    pub fn try_emit(&self, event: TransportTraceEvent) -> bool {
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(_) => {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    pub fn stats(&self) -> TransportTraceStats {
        self.stats.clone()
    }
}

/// 事件泵依赖聚合（泛型 ports + 配置）。
///
/// native / ffi 各自构造此结构（注入自家 EventSink egress），调 `run_engine_loop`。
pub struct EngineDeps<S, H, U, E, C> {
    pub storage: Arc<S>,
    pub http: Arc<H>,
    pub uploader: Arc<U>,
    pub event_sink: Arc<E>,
    pub clock: C,
    pub trace: TraceHooks,
    /// Host 边界共享异步指标出口；热路径只执行有界 `try_record`。
    pub metrics: Arc<dyn AsyncMetricSink>,
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。`0` → `.max(1)` 兜底 1。
    pub max_http_inflight: usize,
    pub transport_lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
    pub transport_trace_tx: Option<TransportTraceSink>,
}
