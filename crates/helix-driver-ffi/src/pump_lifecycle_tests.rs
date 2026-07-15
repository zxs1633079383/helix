use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::SinkExt;
use helix_core::effect::{
    Correlation, DomainEventBytes, FileUploadRequest, HttpRequest, StorageOp, TransportId,
};
use helix_core::tick::AppCommand;
use helix_core::Tick;
use helix_driver_host::trace::TraceHooksImpl;
use helix_driver_host::{
    tick_ingress_channel, AuthTokenRegistry, CommandTraceQueue, HostHeaderRegistry,
    HostNetworkConfig, NoopMetricSink, TraceCarrier, TraceHooks, TraceScope,
};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use super::{run, PumpArgs};
use crate::vtable::CppHostVtable;

const GEN1: &[u8] = b"GEN1";
const GEN2: &[u8] = b"GEN2";
const BARRIER_COMMAND: &str = "__a31_lifecycle_barrier__";

static DB_SEQ: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, PartialEq, Eq)]
enum ObservedTick {
    Connected(u32),
    Inbound(Vec<u8>),
    Disconnected(u32),
    Barrier,
}

struct TickRecorder {
    tx: mpsc::UnboundedSender<ObservedTick>,
}

impl TraceHooksImpl for TickRecorder {
    fn on_tick_start(&self, tick: &Tick, _: Option<TraceCarrier>) -> TraceScope {
        let observed = match tick {
            Tick::Connected(id) => Some(ObservedTick::Connected(id.raw())),
            Tick::Inbound(bytes) => Some(ObservedTick::Inbound(bytes.as_bytes().to_vec())),
            Tick::Disconnected(id) => Some(ObservedTick::Disconnected(id.raw())),
            Tick::Command(command) if command.name == BARRIER_COMMAND => {
                Some(ObservedTick::Barrier)
            }
            _ => None,
        };
        if let Some(observed) = observed {
            self.tx.send(observed).ok();
        }
        TraceScope::noop()
    }

    fn on_storage_dispatch(&self, _: Option<Correlation>, _: &[StorageOp]) {}

    fn on_http_dispatch(&self, _: Option<Correlation>, _: &mut HttpRequest) {}

    fn on_upload_dispatch(&self, _: Option<Correlation>, _: &FileUploadRequest) {}

    fn on_ws_send(&self, _: TransportId, _: &mut Bytes) {}

    fn on_event_emit(&self, _: &DomainEventBytes) {}
}

extern "C" fn noop_events(_: *mut c_void, _: *const u8, _: usize) {}

extern "C" fn fixed_now_ms(_: *mut c_void) -> u64 {
    1_700_000_000_000
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ffi_initial_and_reconnect_publish_connected_before_inbound_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let (release_second_tx, release_second_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (first_stream, _) = listener.accept().await.expect("accept first connection");
        let mut first = accept_async(first_stream)
            .await
            .expect("accept first websocket");
        first
            .send(Message::Binary(GEN1.to_vec()))
            .await
            .expect("send first frame");
        first
            .send(Message::Close(None))
            .await
            .expect("close first connection");

        let (second_stream, _) = listener.accept().await.expect("accept reconnect");
        let mut second = accept_async(second_stream)
            .await
            .expect("accept second websocket");
        second
            .send(Message::Binary(GEN2.to_vec()))
            .await
            .expect("send second frame");
        release_second_rx.await.ok();
        second.send(Message::Close(None)).await.ok();
    });

    let (tick_tx, tick_rx) = tick_ingress_channel(256);
    let barrier_tick_tx = tick_tx.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let (observed_tx, mut observed_rx) = mpsc::unbounded_channel();
    let db_path = std::env::temp_dir().join(format!(
        "helix-a31-ffi-{}-{}.db",
        std::process::id(),
        DB_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let args = PumpArgs {
        config: HostNetworkConfig::new("http://127.0.0.1:1", format!("ws://{addr}/socket")),
        headers: HostHeaderRegistry::default(),
        auth: AuthTokenRegistry::new(),
        command_traces: CommandTraceQueue::default(),
        trace_override: Some(TraceHooks::new(TickRecorder { tx: observed_tx })),
        vtable: CppHostVtable {
            on_domain_events_batch: Some(noop_events),
            now_ms: Some(fixed_now_ms),
            ctx: std::ptr::null_mut(),
        },
        tick_tx,
        tick_rx,
        shutdown_rx,
        ready_tx,
        db_url: db_path.to_string_lossy().into_owned(),
        max_http_inflight: 1,
        metrics: std::sync::Arc::new(NoopMetricSink),
        metric_runtime: None,
    };
    let pump = std::thread::Builder::new()
        .name("helix-a31-ffi-pump".to_string())
        .spawn(move || run(args))
        .expect("spawn pump");

    ready_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("pump should report readiness")
        .expect("pump setup should succeed");

    let expected = [
        ObservedTick::Connected(0),
        ObservedTick::Inbound(GEN1.to_vec()),
        ObservedTick::Disconnected(0),
        ObservedTick::Connected(0),
        ObservedTick::Inbound(GEN2.to_vec()),
    ];
    for expected_tick in expected {
        let actual = timeout(Duration::from_secs(5), observed_rx.recv())
            .await
            .expect("lifecycle tick should arrive")
            .expect("trace recorder should stay open");
        assert_eq!(actual, expected_tick);
    }

    // barrier 与 lifecycle 共用 tick_tx；它被 on_tick_start 观察到时，排在它之前的
    // Connected/Inbound/Disconnected 已全部经过 engine，而非仅在生产者侧排队。
    barrier_tick_tx
        .send(Tick::Command(AppCommand::new(
            BARRIER_COMMAND,
            Bytes::new(),
        )))
        .await
        .expect("send lifecycle queue barrier");
    let barrier = timeout(Duration::from_secs(5), observed_rx.recv())
        .await
        .expect("lifecycle queue barrier should be processed")
        .expect("trace recorder should stay open");
    assert_eq!(barrier, ObservedTick::Barrier);

    shutdown_tx.send(()).ok();
    tokio::task::spawn_blocking(move || pump.join().expect("pump should stop"))
        .await
        .expect("pump join task");
    release_second_tx.send(()).ok();
    timeout(Duration::from_secs(2), server)
        .await
        .expect("server should stop")
        .expect("server task should not panic");

    let mut trailing = Vec::new();
    while let Some(tick) = observed_rx.recv().await {
        trailing.push(tick);
    }
    assert!(
        trailing.is_empty(),
        "pump shutdown barrier 后不得出现延迟的重复生命周期 Tick，实际：{trailing:?}"
    );

    std::fs::remove_file(&db_path).ok();
    std::fs::remove_file(db_path.with_extension("db-wal")).ok();
    std::fs::remove_file(db_path.with_extension("db-shm")).ok();
}
