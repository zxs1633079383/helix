//! Shared host driver pieces for PC Tauri and Flutter FFI.

pub mod batch_sink;
pub mod codec;
pub mod dispatch;
pub mod engine;
pub mod http_cross;
pub mod http_envelope;
pub mod instrument;
pub mod local_store;
pub mod metrics;
pub mod network;
mod network_debug;
#[cfg(test)]
mod network_lifecycle_tests;
pub mod ops;
pub mod otel;
pub mod owned_effect;
pub mod pools;
pub mod reconnect;
pub mod spawner;
pub mod storage;
mod tick_ingress;
pub mod timer;
pub mod trace;
pub mod upload;
pub mod upsert_sql;

pub use batch_sink::BatchSink;
pub use codec::{rows_from_reply_bytes, rows_to_reply_bytes};
pub use engine::{
    register_transport, run_engine_loop, run_engine_loop_stamped, EngineDeps,
    TransportRegistration, TransportTable, TransportTraceSink,
};
pub use http_cross::{AuthTokenRegistry, CrossCuttingHttp, HttpOutcomeObserver};
pub use http_envelope::{base64_encode, run_http_envelope};
pub use instrument::RecordingSink;
pub use local_store::{
    BackendScope, LocalStoreError, LocalStoreIdentity, LocalStoreLocator, LocalStorePaths,
};
pub use metrics::{
    production_metric_ids, AsyncMetricSink, BatchMetricExporter, BoundedMetricRuntime, LabelKey,
    MetricAvailability, MetricEvent, MetricExporterState, MetricId, MetricKind, MetricLabel,
    MetricLabels, MetricRuntimeConfig, MetricRuntimeSnapshot, NoopMetricSink, RecordOutcome,
    ShutdownOutcome, ALLOWED_LABEL_KEYS, CANARY_METRIC_IDS, REQUIRED_METRIC_IDS,
};
pub use network::{
    HostHeaderRegistry, HostNetworkConfig, PreparedHttpRequest, SharedHttpClient, SharedWsClient,
    WsConnectionActivation,
};
pub use ops::{classify_port_error, execute_storage_ops, table_set_key};
pub use otel::{HostOtelConfig, HostOtelRuntime, HostSpanSnapshot, TraceDirection};
pub use owned_effect::{consume_effects, OwnedEffect};
pub use reconnect::{spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink};
pub use spawner::{BoundedSpawner, Job, Overflow};
pub use storage::HostStorage;
pub use tick_ingress::{tick_ingress_channel, TickIngressReceiver, TickIngressSender};
pub use timer::TimerRegistry;
pub use trace::{
    CommandTraceQueue, TraceCarrier, TraceCarrierError, TraceHooks, TraceHooksImpl, TraceScope,
};
pub use upload::SharedFileUploader;

/// per-table-set Persist worker 的有界 job 队列深度上限。
///
/// 超过此值时 dispatch 侧 `.await` 阻塞主循环，将背压传导到 tick_tx（cap=256）。
/// 这恢复了旧实现"磁盘慢则入站慢"的背压链，同时避免死锁（PortReply 走独立通道）。
pub const PERSIST_QUEUE_CAP: usize = 1024;

/// Http / HttpFire BoundedSpawner 的有界队列深度上限。
///
/// 必达 Http（Block）满时 `send().await` 把背压传导回 tick_tx；
/// HttpFire（DropNewest）满时 try_send 失败即丢当前条目（满即丢最新来的）+ warn（自愈靠 cursor-gate 重发）。
pub const HTTP_QUEUE_CAP: usize = 1024;
