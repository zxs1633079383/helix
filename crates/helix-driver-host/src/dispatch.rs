//! dispatch — 一批 OwnedEffect 的分流兑现（泵核子步骤，从 engine 拆出）。
//!
//! 泵线程绝不在此 await worker 完成（抽水在独立 worker，submit 即返回；死锁红线，不变量2）。
//! Persist/Http 走 BoundedSpawner submit（Block 满时背压传导回 tick_tx）；
//! Send inline await（WS send 快）；Emit/Timer 同步；Request 当前 deferred stub。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::mpsc;

use helix_core::effect::{FileUploadRequest, HttpRequest, StorageOp};
use helix_core::ports::{EventSink, Storage, Transport};
use helix_core::Tick;

use crate::engine::TransportTable;
use crate::owned_effect::OwnedEffect;
use crate::pools::get_or_spawn_persist;
use crate::spawner::{BoundedSpawner, Job};
use crate::table_set_key;
use crate::timer::TimerRegistry;
use crate::trace::TraceHooks;

/// 把一批 OwnedEffect 分流兑现（async，BoundedSpawner submit，Block 满时等待）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_effects<S, E, Tr>(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<S>,
    http_pool: &BoundedSpawner<HttpRequest>,
    upload_pool: &BoundedSpawner<FileUploadRequest>,
    http_fire_pool: &BoundedSpawner<HttpRequest>,
    event_sink: &Arc<E>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &mpsc::UnboundedSender<Tick>,
    persist_workers: &mut HashMap<String, BoundedSpawner<Vec<StorageOp>>>,
    transports: &TransportTable<Tr>,
    trace: &TraceHooks,
) where
    S: Storage + Send + Sync + 'static,
    E: EventSink,
    Tr: Transport + Send + 'static,
{
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
                trace.on_storage_dispatch(Some(corr), &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                // Block submit：队列满时 .await 阻塞主循环，背压传导到 tick_tx（不变量2）。
                sp.submit(Job {
                    corr: Some(corr),
                    payload: ops,
                })
                .await;
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                trace.on_storage_dispatch(None, &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                sp.submit(Job {
                    corr: None,
                    payload: ops,
                })
                .await;
            }

            // ── Http（必达，Block 溢出，带 corr 回报，走 reply_tx）──────────
            OwnedEffect::Http { corr, mut req } => {
                trace.on_http_dispatch(Some(corr), &mut req);
                http_pool
                    .submit(Job {
                        corr: Some(corr),
                        payload: req,
                    })
                    .await;
            }

            // ── UploadFile（必达，Block 溢出，带 corr 回报，走 reply_tx）────────
            OwnedEffect::UploadFile { corr, req } => {
                trace.on_upload_dispatch(Some(corr), &req);
                upload_pool
                    .submit(Job {
                        corr: Some(corr),
                        payload: req,
                    })
                    .await;
            }

            // ── HttpFire（可丢，DropNewest 溢出，无 corr 不回报）────────────
            OwnedEffect::HttpFire { mut req } => {
                trace.on_http_dispatch(None, &mut req);
                http_fire_pool
                    .submit(Job {
                        corr: None,
                        payload: req,
                    })
                    .await;
            }

            // ── Send（查路由表 → Transport::send 真发 WS）──────────────────
            // inline await（WS send 快）。失败/未注册只 warn，绝不让泵崩。
            OwnedEffect::Send {
                transport,
                mut frame,
            } => match transports.get(&transport) {
                Some(t) => {
                    trace.on_ws_send(transport, &mut frame);
                    // 表内 `Arc<Tr>`（无外层锁）→ 直接 `t.send()`（`&self`）。并发 send 由
                    // transport 内部 state 锁串行化（既有锁，非本路径新增 per-Send 锁，守 HX-C005）。
                    if let Err(e) = t.send(frame).await {
                        tracing::warn!(
                            transport_id = transport.raw(),
                            error = %e,
                            "Effect::Send 发送失败（连接异常，待重连/补偿兜底）"
                        );
                    }
                }
                None => {
                    tracing::warn!(
                        transport_id = transport.raw(),
                        frame_len = frame.len(),
                        "Effect::Send 无对应 transport（路由表未注册该 id）"
                    );
                }
            },

            // ── Request（EventBus REP·方向①：发起端）─────────────────────
            // TODO(方向①收尾): 接入 HostRequester port。当前为 deferred stub——仅日志、不回灌。
            OwnedEffect::Request {
                corr,
                kind,
                payload,
            } => {
                tracing::warn!(
                    corr = corr.raw(),
                    kind,
                    payload_len = payload.len(),
                    "Effect::Request 尚未接入 HostRequester port（方向① deferred）"
                );
            }

            // ── Emit（同步非阻塞）─────────────────────────────────────────
            OwnedEffect::Emit { event } => {
                trace.on_event_emit(&event);
                event_sink.emit(event);
            }

            // ── ScheduleTimer（timer fires → 往 tick_tx 有界发送）─────────
            OwnedEffect::ScheduleTimer { id, after_ms } => {
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                timer_registry.cancel(id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use bytes::Bytes;
    use parking_lot::Mutex;
    use serde_json::Value;
    use tokio::sync::mpsc;

    use helix_core::effect::{
        BatchDeleteSpec, BatchUpdateSpec, Correlation, DomainEventBytes, FileUploadRequest,
        FileUploadResponse, GetSpec, GuardedBumpSpec, HttpRequest, MonotonicUpsertSpec, Row,
        ScanSpec, StorageOp, TransportId, UpsertSpec,
    };
    use helix_core::ports::{EventSink, FileUploader, Storage, Transport};
    use helix_core::tick::{PortOutcome, ReplyBytes, Tick};
    use helix_core::PortError;

    use crate::pools::spawn_upload_pool;
    use crate::spawner::Overflow;
    use crate::trace::{TraceCarrier, TraceHooksImpl, TraceScope};

    use super::*;

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

    struct DummyTransport;

    #[async_trait::async_trait]
    impl Transport for DummyTransport {
        async fn connect(&mut self) -> Result<(), PortError> {
            unreachable!("transport should not be used in upload dispatch test")
        }

        async fn send(&self, _: Bytes) -> Result<(), PortError> {
            unreachable!("transport should not be used in upload dispatch test")
        }

        async fn recv(&self) -> Result<Option<Bytes>, PortError> {
            unreachable!("transport should not be used in upload dispatch test")
        }

        async fn close(&self) -> Result<(), PortError> {
            unreachable!("transport should not be used in upload dispatch test")
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
            url: "https://oss.example.com/upload".to_string(),
            headers: vec![("x-oss-meta".to_string(), "1".to_string())],
            content_type: Some("image/png".to_string()),
            size: Some(12),
        }
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
            |_req: HttpRequest| async move {
                panic!("http pool should stay unused in upload dispatch test")
            },
        );
        let http_fire_pool: BoundedSpawner<HttpRequest> = BoundedSpawner::new(
            1,
            1,
            Overflow::DropNewest,
            reply_tx.clone(),
            |_req: HttpRequest| async move {
                panic!("http fire pool should stay unused in upload dispatch test")
            },
        );
        let upload_pool = spawn_upload_pool(
            1,
            Arc::new(StubUploader {
                response: FileUploadResponse {
                    object_key: "im/uploads/file.png".to_string(),
                    url: Some("https://cdn.example.com/im/uploads/file.png".to_string()),
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
        let mut persist_workers: HashMap<String, BoundedSpawner<Vec<StorageOp>>> = HashMap::new();
        let transports: TransportTable<DummyTransport> = HashMap::new();
        let corr = Correlation::from_raw(88);

        dispatch_effects(
            vec![OwnedEffect::UploadFile {
                corr,
                req: make_upload_request(),
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
            &trace,
        )
        .await;

        let reply = reply_rx.recv().await.expect("upload should emit a PortReply");

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
            }
            Tick::PortReply { outcome, .. } => panic!("expected Ok upload reply, got {outcome:?}"),
            other => panic!("expected Tick::PortReply, got {other:?}"),
        }
    }
}
