use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, DomainEventBytes, FileUploadRequest, FileUploadResponse,
    FileUploadUrls, GetSpec, GuardedBumpSpec, HttpRequest, HttpResponse, MonotonicUpsertSpec, Row,
    ScanSpec, TransportId, UpsertSpec,
};
use helix_core::ports::{Clock, EventSink, FileUploader, FrameSender, HttpRequester, Storage};
use helix_core::{
    CoreError, Correlation, Effect, EffectSink, ExecutionShell, Module, PortError, Tick,
};
use helix_driver_host::engine::{
    register_transport, run_engine_loop, EngineDeps, TransportLifecycleEvent,
    TransportRegistration, TransportTraceEvent, TransportTraceSink,
};
use helix_driver_host::upload::{
    SharedFileUploader, UploadResponseObservation, UploadResponseObserver,
};
use helix_driver_host::{
    AsyncMetricSink, BatchSink, MetricEvent, MetricId, NoopMetricSink, RecordOutcome, TraceHooks,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct RecordingMetricSink(Arc<Mutex<Vec<MetricEvent>>>);

impl AsyncMetricSink for RecordingMetricSink {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

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

struct UnusedFrameSender;

#[async_trait::async_trait]
impl FrameSender for UnusedFrameSender {
    async fn send(&self, _: Bytes) -> Result<(), PortError> {
        unreachable!("frame sender should not be used")
    }
}

struct RecordingFrameSender {
    tx: mpsc::UnboundedSender<Bytes>,
}

#[async_trait::async_trait]
impl FrameSender for RecordingFrameSender {
    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        self.tx
            .send(frame)
            .map_err(|_| PortError::Transport("recording receiver closed".to_string()))
    }
}

struct SendOnConnected {
    transport_id: TransportId,
}

impl Module for SendOnConnected {
    fn name(&self) -> &'static str {
        "send-on-connected"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::Connected(id) if *id == self.transport_id)
    }

    fn handle(&mut self, _: &Tick, _: u64, out: &mut EffectSink) -> Result<(), CoreError> {
        out.push(Effect::Send {
            transport: self.transport_id,
            frame: Bytes::from_static(br#"{"action":"ping"}"#),
        });
        Ok(())
    }
}

#[tokio::test]
async fn disconnected_tick_emits_lifecycle_and_trace_events() {
    let transport_id = TransportId::from_raw(42);
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(8);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (_transport_tx, transport_rx) =
        mpsc::unbounded_channel::<TransportRegistration<UnusedFrameSender>>();
    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();
    let tick_tx_for_loop = tick_tx.clone();
    let metrics = RecordingMetricSink::default();
    let recorded_metrics = Arc::clone(&metrics.0);

    let deps = EngineDeps {
        storage: Arc::new(UnusedStorage),
        http: Arc::new(UnusedHttp),
        uploader: Arc::new(UnusedUploader),
        event_sink: Arc::new(NoopSink),
        clock: FixedClock,
        trace: TraceHooks::noop(),
        metrics: Arc::new(metrics),
        max_http_inflight: 1,
        transport_lifecycle_tx: Some(lifecycle_tx),
        transport_trace_tx: Some(trace_tx),
    };

    let loop_handle = tokio::spawn(async move {
        run_engine_loop::<_, _, _, _, UnusedFrameSender, _>(
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
    let metric_ids: Vec<_> = recorded_metrics
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.id)
        .collect();
    assert!(metric_ids.contains(&MetricId::TickQueueDepth));
    assert!(metric_ids.contains(&MetricId::CoreStepDurationSeconds));
    assert!(metric_ids.contains(&MetricId::EffectsPerTick));
}

#[tokio::test]
async fn registration_ack_makes_sender_visible_to_connected_effect() {
    let target_id = TransportId::from_raw(2);
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(8);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let (transport_tx, transport_rx) = mpsc::unbounded_channel();
    let (other_frame_tx, mut other_frame_rx) = mpsc::unbounded_channel();
    let (target_frame_tx, mut target_frame_rx) = mpsc::unbounded_channel();

    let deps = EngineDeps {
        storage: Arc::new(UnusedStorage),
        http: Arc::new(UnusedHttp),
        uploader: Arc::new(UnusedUploader),
        event_sink: Arc::new(NoopSink),
        clock: FixedClock,
        trace: TraceHooks::noop(),
        metrics: Arc::new(NoopMetricSink),
        max_http_inflight: 1,
        transport_lifecycle_tx: None,
        transport_trace_tx: None,
    };
    let mut shell = ExecutionShell::new();
    shell.register(SendOnConnected {
        transport_id: target_id,
    });
    let tick_tx_for_loop = tick_tx.clone();
    let loop_handle = tokio::spawn(async move {
        run_engine_loop(
            shell,
            tick_rx,
            tick_tx_for_loop,
            deps,
            shutdown_rx,
            HashMap::new(),
            transport_rx,
        )
        .await;
    });

    register_transport(
        &transport_tx,
        TransportId::from_raw(1),
        Arc::new(RecordingFrameSender { tx: other_frame_tx }),
    )
    .await
    .expect("engine should ack non-target sender after insert");
    register_transport(
        &transport_tx,
        target_id,
        Arc::new(RecordingFrameSender {
            tx: target_frame_tx,
        }),
    )
    .await
    .expect("engine should ack target sender after insert");

    tick_tx
        .send(Tick::Connected(target_id))
        .await
        .expect("publish connected tick after registration ack");

    let frame = tokio::time::timeout(Duration::from_secs(3), target_frame_rx.recv())
        .await
        .expect("target sender should receive frame")
        .expect("target sender channel should stay open");
    assert_eq!(frame, Bytes::from_static(br#"{"action":"ping"}"#));
    assert!(other_frame_rx.try_recv().is_err());

    shutdown_tx.send(()).ok();
    tokio::time::timeout(Duration::from_secs(1), loop_handle)
        .await
        .expect("engine loop should stop after shutdown")
        .expect("engine loop task should not panic");
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UploadFact {
    method: &'static str,
    status: Option<u16>,
    key: String,
    bytes: u64,
}

#[derive(Default)]
struct UploadDeliveryState {
    observed: Vec<UploadFact>,
    pending: Vec<UploadFact>,
    delivered: Vec<UploadFact>,
    flush_calls: usize,
    observed_at_flush: Vec<usize>,
}

#[derive(Default)]
struct UploadDiagnosticSink(Mutex<UploadDeliveryState>);

impl UploadResponseObserver for UploadDiagnosticSink {
    fn observe(&self, observation: UploadResponseObservation<'_>) {
        let fact = UploadFact {
            method: observation.method,
            status: observation.http_status,
            key: observation.object_key.to_owned(),
            bytes: observation.file_size,
        };
        let mut state = self.0.lock().unwrap();
        let flush_calls = state.flush_calls;
        state.observed_at_flush.push(flush_calls);
        state.observed.push(fact.clone());
        state.pending.push(fact);
    }
}

impl EventSink for UploadDiagnosticSink {
    fn emit(&self, _: DomainEventBytes) {
        unreachable!("this lifecycle emits only upload diagnostics");
    }
}

impl BatchSink for UploadDiagnosticSink {
    fn flush(&self) {
        let mut state = self.0.lock().unwrap();
        state.flush_calls += 1;
        let pending = std::mem::take(&mut state.pending);
        state.delivered.extend(pending);
    }
}

struct UploadOnStart {
    request: FileUploadRequest,
    replies: Arc<Mutex<usize>>,
}

impl Module for UploadOnStart {
    fn name(&self) -> &'static str {
        "shutdown-upload"
    }

    fn accepts(&self, tick: &Tick) -> bool {
        matches!(tick, Tick::PortReply { corr, .. } if corr.raw() == 91)
    }

    fn on_start(&mut self, out: &mut EffectSink) -> Result<(), CoreError> {
        out.push(Effect::UploadFile {
            corr: Correlation::from_raw(91),
            req: self.request.clone(),
        });
        Ok(())
    }

    fn handle(&mut self, _: &Tick, _: u64, _: &mut EffectSink) -> Result<(), CoreError> {
        *self.replies.lock().unwrap() += 1;
        Ok(())
    }
}

#[tokio::test]
async fn shutdown_drain_flushes_upload_response_diagnostics() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let payload = b"shutdown payload";
    let (request_tx, request_rx) = oneshot::channel();
    let (response_tx, response_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut chunk = [0; 4096];
        let header_end = loop {
            let read = socket.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "PUT must contain its full request");
            request.extend_from_slice(&chunk[..read]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
        };
        let headers = std::str::from_utf8(&request[..header_end]).unwrap();
        assert!(headers.starts_with("PUT /shutdown-upload HTTP/1.1"));
        let length: usize = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().unwrap())
            })
            .unwrap();
        assert_eq!(length, payload.len());
        while request.len() < header_end + length {
            let read = socket.read(&mut chunk).await.unwrap();
            assert_ne!(read, 0, "PUT body must reach the local server");
            request.extend_from_slice(&chunk[..read]);
        }
        assert_eq!(&request[header_end..], payload);
        request_tx.send(()).unwrap();
        response_rx.await.unwrap();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let local_path = std::env::temp_dir().join(format!(
        "helix-shutdown-upload-{}-{}.bin",
        std::process::id(),
        address.port()
    ));
    tokio::fs::write(&local_path, payload).await.unwrap();
    let sink = Arc::new(UploadDiagnosticSink::default());
    let replies = Arc::new(Mutex::new(0));
    let mut shell = ExecutionShell::new();
    shell.register(UploadOnStart {
        request: FileUploadRequest {
            local_path: local_path.to_string_lossy().into_owned(),
            object_key: "shutdown/object.bin".to_owned(),
            method: "PUT".to_owned(),
            urls: FileUploadUrls::new(
                format!("http://{address}/shutdown-upload"),
                "https://cdn.example.test/shutdown/object.bin".to_owned(),
            )
            .unwrap(),
            headers: vec![],
            content_type: None,
            size: Some(payload.len() as u64),
        },
        replies: replies.clone(),
    });
    let (tick_tx, tick_rx) = mpsc::channel(8);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (_transport_tx, transport_rx) = mpsc::unbounded_channel();
    let deps = EngineDeps {
        storage: Arc::new(UnusedStorage),
        http: Arc::new(UnusedHttp),
        uploader: Arc::new(
            SharedFileUploader::new(reqwest::Client::builder().no_proxy().build().unwrap())
                .with_observer(sink.clone()),
        ),
        event_sink: sink.clone(),
        clock: FixedClock,
        trace: TraceHooks::noop(),
        metrics: Arc::new(NoopMetricSink),
        max_http_inflight: 1,
        transport_lifecycle_tx: None,
        transport_trace_tx: None,
    };
    let engine = run_engine_loop::<_, _, _, _, UnusedFrameSender, _>(
        shell,
        tick_rx,
        tick_tx.clone(),
        deps,
        shutdown_rx,
        HashMap::new(),
        transport_rx,
    );
    tokio::pin!(engine);
    tokio::select! {
        _ = engine.as_mut() => panic!("engine must stay alive while PUT is held"),
        request = tokio::time::timeout(Duration::from_secs(3), request_rx) => {
            request.expect("real PUT should reach server").unwrap();
        }
    }
    tick_tx
        .send(Tick::Connected(TransportId::from_raw(99)))
        .await
        .unwrap();
    shutdown_tx.send(()).unwrap();
    // One synchronous poll consumes the ready shutdown and reaches the held upload drain.
    assert!(futures_util::poll!(engine.as_mut()).is_pending());
    assert_eq!(
        tick_tx.capacity(),
        7,
        "shutdown must win over queued ingress"
    );
    let flushes_before_response = {
        let state = sink.0.lock().unwrap();
        assert!(state.observed.is_empty());
        assert!(state.delivered.is_empty());
        state.flush_calls
    };
    response_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), engine.as_mut())
        .await
        .expect("engine should drain the completed PUT");
    server.await.expect("real PUT server should finish");
    tokio::fs::remove_file(local_path).await.unwrap();

    let state = sink.0.lock().unwrap();
    let expected = vec![UploadFact {
        method: "PUT",
        status: Some(204),
        key: "shutdown/object.bin".to_owned(),
        bytes: payload.len() as u64,
    }];
    assert_eq!(
        *replies.lock().unwrap(),
        0,
        "drain must not run a reply step"
    );
    assert_eq!(
        state.observed, expected,
        "observer must see the actual PUT response"
    );
    assert_eq!(state.observed_at_flush, vec![flushes_before_response]);
    assert_eq!(
        state.delivered, expected,
        "drain diagnostics must reach the sink before return"
    );
    assert!(
        state.pending.is_empty(),
        "no upload diagnostic may remain queued"
    );
    assert_eq!(
        state.flush_calls,
        flushes_before_response + 1,
        "drain adds exactly one final boundary"
    );
}
