use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

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
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。`0` → `.max(1)` 兜底 1。
    pub max_http_inflight: usize,
    pub transport_lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
    pub transport_trace_tx: Option<mpsc::UnboundedSender<TransportTraceEvent>>,
}
