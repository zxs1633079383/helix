use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, DomainEventBytes, FileUploadRequest, FileUploadResponse,
    GetSpec, GuardedBumpSpec, HttpRequest, HttpResponse, MonotonicUpsertSpec, Row, ScanSpec,
    TransportId, UpsertSpec,
};
use helix_core::ports::{Clock, EventSink, FileUploader, HttpRequester, Storage, Transport};
use helix_core::{ExecutionShell, PortError, Tick};
use helix_driver_host::engine::{
    run_engine_loop, EngineDeps, TransportLifecycleEvent, TransportTraceEvent, TransportTraceSink,
};
use helix_driver_host::{BatchSink, TraceHooks};

struct UnusedStorage;

#[async_trait::async_trait]
impl Storage for UnusedStorage {
    async fn batch_upsert(&self, _: UpsertSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used")
    }

    async fn batch_update(&self, _: BatchUpdateSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used")
    }

    async fn monotonic_upsert(&self, _: MonotonicUpsertSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used")
    }

    async fn guarded_bump(&self, _: GuardedBumpSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used")
    }

    async fn get(&self, _: GetSpec) -> Result<Option<Row>, PortError> {
        unreachable!("storage should not be used")
    }

    async fn scan(&self, _: ScanSpec) -> Result<Vec<Row>, PortError> {
        unreachable!("storage should not be used")
    }

    async fn batch_delete(&self, _: BatchDeleteSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used")
    }
}

struct UnusedHttp;

#[async_trait::async_trait]
impl HttpRequester for UnusedHttp {
    async fn request(&self, _: HttpRequest) -> Result<HttpResponse, PortError> {
        unreachable!("http should not be used")
    }
}

struct UnusedUploader;

#[async_trait::async_trait]
impl FileUploader for UnusedUploader {
    async fn upload(&self, _: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        unreachable!("uploader should not be used")
    }
}

struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _: DomainEventBytes) {}
}

impl BatchSink for NoopSink {}

#[derive(Clone)]
struct FixedClock;

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        1_725_000_000_000
    }
}

struct UnusedTransport;

#[async_trait::async_trait]
impl Transport for UnusedTransport {
    async fn connect(&mut self) -> Result<(), PortError> {
        unreachable!("transport should not be used")
    }

    async fn send(&self, _: Bytes) -> Result<(), PortError> {
        unreachable!("transport should not be used")
    }

    async fn recv(&self) -> Result<Option<Bytes>, PortError> {
        unreachable!("transport should not be used")
    }

    async fn close(&self) -> Result<(), PortError> {
        unreachable!("transport should not be used")
    }
}

#[tokio::test]
async fn disconnected_tick_emits_lifecycle_and_trace_events() {
    let transport_id = TransportId::from_raw(42);
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(8);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (_transport_tx, transport_rx) = mpsc::unbounded_channel();
    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();
    let tick_tx_for_loop = tick_tx.clone();

    let deps = EngineDeps {
        storage: Arc::new(UnusedStorage),
        http: Arc::new(UnusedHttp),
        uploader: Arc::new(UnusedUploader),
        event_sink: Arc::new(NoopSink),
        clock: FixedClock,
        trace: TraceHooks::noop(),
        max_http_inflight: 1,
        transport_lifecycle_tx: Some(lifecycle_tx),
        transport_trace_tx: Some(trace_tx),
    };

    let loop_handle = tokio::spawn(async move {
        run_engine_loop::<_, _, _, _, UnusedTransport, _>(
            ExecutionShell::new(),
            tick_rx,
            tick_tx_for_loop,
            deps,
            shutdown_rx,
            HashMap::new(),
            transport_rx,
        )
        .await;
    });

    tick_tx
        .send(Tick::Disconnected(transport_id))
        .await
        .expect("send disconnected tick");

    let lifecycle = tokio::time::timeout(Duration::from_millis(500), lifecycle_rx.recv())
        .await
        .expect("lifecycle event should arrive")
        .expect("lifecycle channel should stay open");
    assert_eq!(
        lifecycle,
        TransportLifecycleEvent::Disconnected {
            transport_id,
            reason: "reader_closed_or_error",
        }
    );

    let trace = tokio::time::timeout(Duration::from_millis(500), trace_rx.recv())
        .await
        .expect("trace event should arrive")
        .expect("trace channel should stay open");
    assert_eq!(
        trace,
        TransportTraceEvent {
            transport_id,
            name: "helix.ws.disconnect",
            action: "disconnect",
            attempt: None,
            delay_ms: None,
            next_delay_ms: None,
            reason: Some("reader_closed_or_error"),
            error_class: None,
        }
    );

    shutdown_tx.send(()).ok();
    tokio::time::timeout(Duration::from_secs(1), loop_handle)
        .await
        .expect("engine loop should stop after shutdown")
        .expect("engine loop task should not panic");
}
