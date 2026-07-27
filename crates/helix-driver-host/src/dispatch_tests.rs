use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};

use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, Correlation, DomainEventBytes, FileUploadProgress,
    FileUploadProgressPolicy, FileUploadRequest, FileUploadResponse, FileUploadUrls, GetSpec,
    GuardedBumpSpec, HttpRequest, MonotonicUpsertSpec, Row, ScanSpec, StorageOp, TransportId,
    UpsertSpec,
};
use helix_core::ports::{
    EventSink, FileUploadProgressReporter, FileUploader, FrameSender, Storage,
};
use helix_core::tick::{PortOutcome, ReplyBytes, Tick};
use helix_core::PortError;

use crate::engine::TransportTable;
use crate::owned_effect::OwnedEffect;
use crate::pools::spawn_upload_pool;
use crate::spawner::{BoundedSpawner, Job, Overflow};
use crate::timer::TimerRegistry;
use crate::trace::{TraceCarrier, TraceHooks, TraceHooksImpl, TraceScope};
use crate::SharedFileUploader;

use super::dispatch_effects;
use crate::engine::perf_metrics::EngineMetricRecorder;
use crate::metrics::NoopMetricSink;
use crate::tick_ingress::EngineTickSender;

struct UnusedStorage;

#[async_trait::async_trait]
impl Storage for UnusedStorage {
    async fn batch_upsert(&self, _: UpsertSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn batch_update(&self, _: BatchUpdateSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn monotonic_upsert(&self, _: MonotonicUpsertSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn guarded_bump(&self, _: GuardedBumpSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn get(&self, _: GetSpec) -> Result<Option<Row>, PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn scan(&self, _: ScanSpec) -> Result<Vec<Row>, PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }

    async fn batch_delete(&self, _: BatchDeleteSpec) -> Result<(), PortError> {
        unreachable!("storage should not be used in upload dispatch test")
    }
}

struct NoopEventSink;

impl EventSink for NoopEventSink {
    fn emit(&self, _: DomainEventBytes) {}
}

struct StubUploader {
    response: FileUploadResponse,
}

#[async_trait::async_trait]
impl FileUploader for StubUploader {
    async fn upload(&self, _: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        Ok(self.response.clone())
    }
}

struct ProgressUploader {
    response: FileUploadResponse,
}

#[async_trait::async_trait]
impl FileUploader for ProgressUploader {
    async fn upload(&self, _: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        Ok(self.response.clone())
    }

    async fn upload_with_progress(
        &self,
        _: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        if let Some(progress) = progress {
            for completed_bytes in 1..=100 {
                progress.report(FileUploadProgress {
                    completed_bytes,
                    total_bytes: 100,
                });
            }
        }
        Ok(self.response.clone())
    }
}

struct GatedRealUploader {
    inner: SharedFileUploader,
    first_wave_gate: Arc<Semaphore>,
    started_tx: mpsc::UnboundedSender<usize>,
    started_total: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    max_active: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl FileUploader for GatedRealUploader {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        self.upload_with_progress(req, None).await
    }

    async fn upload_with_progress(
        &self,
        req: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        let ordinal = self.started_total.fetch_add(1, Ordering::SeqCst) + 1;
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        self.started_tx
            .send(ordinal)
            .expect("test start observer must remain alive");

        // 前八个 worker 在进入真实 HTTP 前全部停住。只有池中确有八个独立 worker，
        // 测试端才能收到八个 started；此时第九个 job 结构上只能仍在有界队列中。
        if ordinal <= 8 {
            let permit = self
                .first_wave_gate
                .acquire()
                .await
                .expect("test upload gate must remain open");
            drop(permit);
        }

        let result = self.inner.upload_with_progress(req, progress).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }
}

const NINE_UPLOAD_BODY: &[u8] = b"nine-image-payload";

async fn accept_local_put(mut socket: TcpStream) -> String {
    let mut request = Vec::new();
    let expected_total = loop {
        if let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("real PUT must carry Content-Length");
            let expected_total = header_end + content_length;
            if request.len() >= expected_total {
                break expected_total;
            }
        }

        let mut chunk = [0_u8; 4096];
        let read = socket
            .read(&mut chunk)
            .await
            .expect("read local PUT request");
        assert!(
            read > 0,
            "local PUT closed before its declared body arrived"
        );
        request.extend_from_slice(&chunk[..read]);
    };

    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .expect("request headers must be complete");
    assert_eq!(
        &request[header_end..expected_total],
        NINE_UPLOAD_BODY,
        "local server must receive the complete streamed file body"
    );
    let request_line = String::from_utf8_lossy(&request[..header_end])
        .lines()
        .next()
        .expect("request line")
        .to_string();
    assert!(request_line.starts_with("PUT "));

    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nETag: \"local-etag\"\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("write local PUT response");
    request_line
}

#[derive(Default)]
struct RecordingFrameSender {
    frames: Mutex<Vec<Bytes>>,
}

#[async_trait::async_trait]
impl FrameSender for RecordingFrameSender {
    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        self.frames.lock().push(frame);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingTraceHooks {
    uploads: Arc<Mutex<Vec<(Option<u64>, String)>>>,
}

impl TraceHooksImpl for RecordingTraceHooks {
    fn on_tick_start(&self, _: &Tick, _: Option<TraceCarrier>) -> TraceScope {
        TraceScope::noop()
    }

    fn on_storage_dispatch(&self, _: Option<Correlation>, _: &[StorageOp]) {}

    fn on_http_dispatch(&self, _: Option<Correlation>, _: &mut HttpRequest) {}

    fn on_upload_dispatch(&self, corr: Option<Correlation>, req: &FileUploadRequest) {
        self.uploads
            .lock()
            .push((corr.map(Correlation::raw), req.object_key.clone()));
    }

    fn on_ws_send(&self, _: TransportId, _: &mut Bytes) {}

    fn on_event_emit(&self, _: &DomainEventBytes) {}
}

fn make_upload_request() -> FileUploadRequest {
    FileUploadRequest {
        local_path: "/tmp/file.png".to_string(),
        object_key: "im/uploads/file.png".to_string(),
        method: "PUT".to_string(),
        urls: FileUploadUrls::new(
            "https://oss.example.com/upload?signature=write-secret".to_string(),
            "https://cdn.example.com/im/uploads/file.png".to_string(),
        )
        .expect("valid upload urls"),
        headers: vec![("x-oss-meta".to_string(), "1".to_string())],
        content_type: Some("image/png".to_string()),
        size: Some(12),
    }
}

fn make_upload_request_with_policy(policy: FileUploadProgressPolicy) -> FileUploadRequest {
    let mut req = make_upload_request();
    req.urls = req.urls.with_progress_policy(policy);
    req
}

#[tokio::test]
async fn real_http_nine_uploads_respect_eight_inflight_and_hide_signed_queries_from_replies() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local upload server");
    let address = listener.local_addr().expect("local upload address");
    let server = tokio::spawn(async move {
        let mut handlers = Vec::with_capacity(9);
        for _ in 0..9 {
            let (socket, _) = listener.accept().await.expect("accept local PUT");
            handlers.push(tokio::spawn(accept_local_put(socket)));
        }
        let mut request_lines = Vec::with_capacity(9);
        for handler in handlers {
            request_lines.push(handler.await.expect("join local PUT handler"));
        }
        request_lines
    });

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let local_path = std::env::temp_dir().join(format!("helix-nine-upload-{nonce}.png"));
    tokio::fs::write(&local_path, NINE_UPLOAD_BODY)
        .await
        .expect("write nine-upload fixture");

    let first_wave_gate = Arc::new(Semaphore::new(0));
    let started_total = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let uploader = Arc::new(GatedRealUploader {
        inner: SharedFileUploader::default().with_timeout(Duration::from_secs(5)),
        first_wave_gate: Arc::clone(&first_wave_gate),
        started_tx,
        started_total: Arc::clone(&started_total),
        active: Arc::clone(&active),
        max_active: Arc::clone(&max_active),
    });
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();
    let upload_pool = spawn_upload_pool(8, uploader, reply_tx, Overflow::Block);

    for index in 0..9_u64 {
        upload_pool
            .submit(Job {
                corr: Some(Correlation::from_raw(900 + index)),
                payload: FileUploadRequest {
                    local_path: local_path.to_string_lossy().into_owned(),
                    object_key: format!("media/image-{index}"),
                    method: "PUT".to_string(),
                    urls: FileUploadUrls::new(
                        format!("http://{address}/upload/{index}?Signature=secret-{index}"),
                        format!("https://java.example.test/oss/media/object/image-{index}"),
                    )
                    .expect("valid local signed/public URLs"),
                    headers: vec![
                        ("Content-Type".to_string(), "image/png".to_string()),
                        ("x-oss-forbid-overwrite".to_string(), "true".to_string()),
                    ],
                    content_type: Some("image/png".to_string()),
                    size: Some(NINE_UPLOAD_BODY.len() as u64),
                },
            })
            .await;
    }

    let mut first_wave = Vec::with_capacity(8);
    for _ in 0..8 {
        let observed = tokio::time::timeout(Duration::from_secs(2), started_rx.recv())
            .await
            .expect("eight upload workers must start promptly")
            .expect("start observer must remain open");
        first_wave.push(observed);
    }
    first_wave.sort_unstable();
    assert_eq!(first_wave, (1..=8).collect::<Vec<_>>());
    assert_eq!(started_total.load(Ordering::SeqCst), 8);
    assert_eq!(active.load(Ordering::SeqCst), 8);
    assert_eq!(max_active.load(Ordering::SeqCst), 8);
    assert!(
        started_rx.try_recv().is_err(),
        "the ninth upload must remain queued while all eight workers are gated"
    );

    first_wave_gate.add_permits(8);
    upload_pool.shutdown().await;
    let request_lines = server.await.expect("join local upload server");

    assert_eq!(started_total.load(Ordering::SeqCst), 9);
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(
        max_active.load(Ordering::SeqCst),
        8,
        "host upload pool must never exceed its configured in-flight limit"
    );
    assert_eq!(request_lines.len(), 9);
    for index in 0..9 {
        assert!(
            request_lines.iter().any(|line| {
                line.contains(&format!("/upload/{index}?Signature=secret-{index}"))
            }),
            "local server must receive signed PUT #{index}"
        );
    }

    let mut terminal_replies = 0;
    let mut correlations = Vec::new();
    while let Ok(tick) = reply_rx.try_recv() {
        match tick {
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(bytes)),
            } => {
                terminal_replies += 1;
                correlations.push(corr.raw());
                let wire = String::from_utf8_lossy(&bytes);
                assert!(!wire.contains("Signature="));
                assert!(!wire.contains("secret-"));
                let json: Value =
                    serde_json::from_slice(&bytes).expect("upload reply must be valid JSON");
                assert!(
                    json["url"]
                        .as_str()
                        .is_some_and(|url| url
                            .starts_with("https://java.example.test/oss/media/object/image-")),
                    "reply must expose only the stable Java URL"
                );
            }
            Tick::PortReply { outcome, .. } => {
                panic!("all nine real uploads must succeed, got {outcome:?}")
            }
            Tick::PortProgress { .. } => {
                panic!("image uploads must not emit percentage progress")
            }
            other => panic!("unexpected upload-pool reply: {other:?}"),
        }
    }
    correlations.sort_unstable();
    assert_eq!(terminal_replies, 9);
    assert_eq!(correlations, (900..909).collect::<Vec<_>>());

    tokio::fs::remove_file(local_path)
        .await
        .expect("remove nine-upload fixture");
}

#[tokio::test]
async fn upload_pool_emits_at_most_twenty_ticks_at_five_percent_steps() {
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();
    let upload_pool = spawn_upload_pool(
        1,
        Arc::new(ProgressUploader {
            response: FileUploadResponse {
                object_key: "im/uploads/file.png".to_string(),
                public_url: "https://cdn.example.com/im/uploads/file.png".to_string(),
                etag: None,
            },
        }),
        reply_tx,
        Overflow::Block,
    );

    upload_pool
        .submit(Job {
            corr: Some(Correlation::from_raw(501)),
            payload: make_upload_request_with_policy(FileUploadProgressPolicy::PercentStep(5)),
        })
        .await;
    upload_pool
        .submit(Job {
            corr: Some(Correlation::from_raw(502)),
            payload: make_upload_request_with_policy(FileUploadProgressPolicy::Disabled),
        })
        .await;
    upload_pool.shutdown().await;

    let mut enabled_progress = Vec::new();
    let mut disabled_progress = 0;
    let mut enabled_terminal = 0;
    let mut disabled_terminal = 0;
    while let Ok(tick) = reply_rx.try_recv() {
        match tick {
            Tick::PortProgress { corr, progress } if corr.raw() == 501 => {
                enabled_progress.push(
                    ((u128::from(progress.completed_bytes) * 100)
                        / u128::from(progress.total_bytes)) as u8,
                );
            }
            Tick::PortProgress { corr, .. } if corr.raw() == 502 => disabled_progress += 1,
            Tick::PortReply { corr, .. } if corr.raw() == 501 => enabled_terminal += 1,
            Tick::PortReply { corr, .. } if corr.raw() == 502 => disabled_terminal += 1,
            _ => {}
        }
    }

    assert_eq!(enabled_progress, (5..=100).step_by(5).collect::<Vec<_>>());
    assert_eq!(disabled_progress, 0);
    assert_eq!(enabled_terminal, 1);
    assert_eq!(disabled_terminal, 1);
}

#[tokio::test]
async fn upload_effect_dispatch_records_trace_and_replies_with_expected_json() {
    let (tick_tx, _tick_rx) = mpsc::channel::<Tick>(1);
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();
    let storage = Arc::new(UnusedStorage);
    let event_sink = Arc::new(NoopEventSink);
    let http_pool: BoundedSpawner<HttpRequest> = BoundedSpawner::new(
        1,
        1,
        Overflow::Block,
        reply_tx.clone(),
        |_corr, _req: HttpRequest| async move {
            panic!("http pool should stay unused in upload dispatch test")
        },
    );
    let http_fire_pool: BoundedSpawner<HttpRequest> = BoundedSpawner::new(
        1,
        1,
        Overflow::DropNewest,
        reply_tx.clone(),
        |_corr, _req: HttpRequest| async move {
            panic!("http fire pool should stay unused in upload dispatch test")
        },
    );
    let upload_pool = spawn_upload_pool(
        1,
        Arc::new(StubUploader {
            response: FileUploadResponse {
                object_key: "im/uploads/file.png".to_string(),
                public_url: "https://cdn.example.com/im/uploads/file.png".to_string(),
                etag: Some("etag-123".to_string()),
            },
        }),
        reply_tx.clone(),
        Overflow::Block,
    );
    let recording = RecordingTraceHooks::default();
    let seen_uploads = Arc::clone(&recording.uploads);
    let trace = TraceHooks::new(recording);
    let mut timer_registry = TimerRegistry::new();
    let mut persist_workers: HashMap<String, BoundedSpawner<crate::pools::PersistJobPayload>> =
        HashMap::new();
    let transports: TransportTable<RecordingFrameSender> = HashMap::new();
    let corr = Correlation::from_raw(88);
    let mut perf_metrics = EngineMetricRecorder::new(Arc::new(NoopMetricSink));

    dispatch_effects(
        vec![OwnedEffect::UploadFile {
            corr,
            req: make_upload_request(),
        }],
        &EngineTickSender::Raw(tick_tx),
        &storage,
        &http_pool,
        &upload_pool,
        &http_fire_pool,
        &event_sink,
        &mut timer_registry,
        &reply_tx,
        &mut persist_workers,
        &transports,
        &trace,
        &mut perf_metrics,
    )
    .await;

    let reply = reply_rx
        .recv()
        .await
        .expect("upload should emit a PortReply");

    upload_pool.shutdown().await;
    http_pool.shutdown().await;
    http_fire_pool.shutdown().await;

    assert_eq!(
        &*seen_uploads.lock(),
        &[(Some(88), "im/uploads/file.png".to_string())]
    );

    match reply {
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes(bytes)),
        } => {
            assert_eq!(corr.raw(), 88);
            let json: Value =
                serde_json::from_slice(&bytes).expect("upload reply should be valid JSON");
            assert_eq!(json["objectKey"].as_str(), Some("im/uploads/file.png"));
            assert_eq!(
                json["url"].as_str(),
                Some("https://cdn.example.com/im/uploads/file.png")
            );
            assert_eq!(json["etag"].as_str(), Some("etag-123"));
            assert!(
                !String::from_utf8_lossy(&bytes).contains("write-secret"),
                "PortReply must not expose the signed PUT URL"
            );
        }
        Tick::PortReply { outcome, .. } => panic!("expected Ok upload reply, got {outcome:?}"),
        other => panic!("expected Tick::PortReply, got {other:?}"),
    }
}

#[tokio::test]
async fn send_effect_requires_only_frame_sender_capability() {
    let (tick_tx, _tick_rx) = mpsc::channel::<Tick>(1);
    let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();
    let storage = Arc::new(UnusedStorage);
    let event_sink = Arc::new(NoopEventSink);
    let http_pool: BoundedSpawner<HttpRequest> = BoundedSpawner::new(
        1,
        1,
        Overflow::Block,
        reply_tx.clone(),
        |_corr, _req: HttpRequest| async move { panic!("http pool should stay unused") },
    );
    let http_fire_pool: BoundedSpawner<HttpRequest> = BoundedSpawner::new(
        1,
        1,
        Overflow::DropNewest,
        reply_tx.clone(),
        |_corr, _req: HttpRequest| async move { panic!("http fire pool should stay unused") },
    );
    let upload_pool = spawn_upload_pool(
        1,
        Arc::new(StubUploader {
            response: FileUploadResponse {
                object_key: "unused".to_string(),
                public_url: "https://example.invalid/file".to_string(),
                etag: None,
            },
        }),
        reply_tx.clone(),
        Overflow::Block,
    );
    let sender = Arc::new(RecordingFrameSender::default());
    let transport_id = TransportId::from_raw(7);
    let mut transports = TransportTable::new();
    transports.insert(transport_id, Arc::clone(&sender));
    let mut timer_registry = TimerRegistry::new();
    let mut persist_workers = HashMap::new();

    let tick_tx = EngineTickSender::Raw(tick_tx);
    let mut perf_metrics = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    dispatch_effects(
        vec![OwnedEffect::Send {
            transport: transport_id,
            frame: Bytes::from_static(br#"{"action":"ping"}"#),
        }],
        &tick_tx,
        &storage,
        &http_pool,
        &upload_pool,
        &http_fire_pool,
        &event_sink,
        &mut timer_registry,
        &reply_tx,
        &mut persist_workers,
        &transports,
        &TraceHooks::noop(),
        &mut perf_metrics,
    )
    .await;

    assert_eq!(
        sender.frames.lock().as_slice(),
        &[Bytes::from_static(br#"{"action":"ping"}"#)]
    );

    upload_pool.shutdown().await;
    http_pool.shutdown().await;
    http_fire_pool.shutdown().await;
}
