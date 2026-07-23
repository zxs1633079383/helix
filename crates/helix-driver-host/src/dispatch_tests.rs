use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc;

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
