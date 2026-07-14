//! dispatch — 一批 OwnedEffect 的分流兑现（泵核子步骤，从 engine 拆出）。
//!
//! 泵线程绝不在此 await worker 完成（抽水在独立 worker，submit 即返回；死锁红线，不变量2）。
//! Persist/Http 走 BoundedSpawner submit（Block 满时背压传导回 tick_tx）；
//! Send inline await（WS send 快）；Emit/Timer 同步；Request 当前 deferred stub。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;

use helix_core::effect::{FileUploadRequest, HttpRequest};
use helix_core::ports::{EventSink, Storage, Transport};
use helix_core::Tick;

use crate::engine::TransportTable;
use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use crate::owned_effect::OwnedEffect;
use crate::pools::{get_or_spawn_persist, PersistJobPayload};
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
    persist_workers: &mut HashMap<String, BoundedSpawner<PersistJobPayload>>,
    transports: &TransportTable<Tr>,
    trace: &TraceHooks,
    metrics: &dyn AsyncMetricSink,
) where
    S: Storage + Send + Sync + 'static,
    E: EventSink,
    Tr: Transport + Send + 'static,
{
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
                record_operation(metrics, "persist", "ok");
                trace.on_storage_dispatch(Some(corr), &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                // Block submit：队列满时 .await 阻塞主循环，背压传导到 tick_tx（不变量2）。
                let queued_at = metrics.is_enabled().then(Instant::now);
                sp.submit(Job {
                    corr: Some(corr),
                    payload: PersistJobPayload {
                        ops,
                        trace: trace.storage_trace_context(),
                    },
                })
                .await;
                record_wait(
                    metrics,
                    MetricId::PersistQueueWaitSeconds,
                    queued_at,
                    "persist",
                );
            }

            // ── PersistFire（fire-and-forget，不产出 PortReply）────────────
            OwnedEffect::PersistFire { ops } => {
                record_operation(metrics, "persist_fire", "ok");
                trace.on_storage_dispatch(None, &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                );
                let queued_at = metrics.is_enabled().then(Instant::now);
                sp.submit(Job {
                    corr: None,
                    payload: PersistJobPayload {
                        ops,
                        trace: trace.storage_trace_context(),
                    },
                })
                .await;
                record_wait(
                    metrics,
                    MetricId::PersistQueueWaitSeconds,
                    queued_at,
                    "persist_fire",
                );
            }

            // ── Http（必达，Block 溢出，带 corr 回报，走 reply_tx）──────────
            OwnedEffect::Http { corr, mut req } => {
                record_operation(metrics, "http", "ok");
                trace.on_http_dispatch(Some(corr), &mut req);
                let queued_at = metrics.is_enabled().then(Instant::now);
                http_pool
                    .submit(Job {
                        corr: Some(corr),
                        payload: req,
                    })
                    .await;
                record_wait(metrics, MetricId::HttpQueueWaitSeconds, queued_at, "http");
            }

            // ── UploadFile（必达，Block 溢出，带 corr 回报，走 reply_tx）────────
            OwnedEffect::UploadFile { corr, req } => {
                record_operation(metrics, "upload_file", "ok");
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
                record_operation(metrics, "http_fire", "ok");
                trace.on_http_dispatch(None, &mut req);
                let queued_at = metrics.is_enabled().then(Instant::now);
                http_fire_pool
                    .submit(Job {
                        corr: None,
                        payload: req,
                    })
                    .await;
                record_wait(
                    metrics,
                    MetricId::HttpQueueWaitSeconds,
                    queued_at,
                    "http_fire",
                );
            }

            // ── Send（查路由表 → Transport::send 真发 WS）──────────────────
            // inline await（WS send 快）。失败/未注册只 warn，绝不让泵崩。
            OwnedEffect::Send {
                transport,
                mut frame,
            } => match transports.get(&transport) {
                Some(t) => {
                    record_operation(metrics, "ws_send", "ok");
                    trace.on_ws_send(transport, &mut frame);
                    // 表内 `Arc<Tr>`（无外层锁）→ 直接 `t.send()`（`&self`）。并发 send 由
                    // transport 内部 state 锁串行化（既有锁，非本路径新增 per-Send 锁，守 HX-C005）。
                    if let Err(e) = t.send(frame).await {
                        record_error(metrics, "ws_send_failed");
                        tracing::warn!(
                            transport_id = transport.raw(),
                            error = %e,
                            "Effect::Send 发送失败（连接异常，待重连/补偿兜底）"
                        );
                    }
                }
                None => {
                    record_error(metrics, "ws_transport_missing");
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
                record_operation(metrics, "request", "deferred");
                tracing::warn!(
                    corr = corr.raw(),
                    kind,
                    payload_len = payload.len(),
                    "Effect::Request 尚未接入 HostRequester port（方向① deferred）"
                );
            }

            // ── Emit（同步非阻塞）─────────────────────────────────────────
            OwnedEffect::Emit { event } => {
                record_operation(metrics, "event_emit", "ok");
                trace.on_event_emit(&event);
                event_sink.emit(event);
            }

            // ── ScheduleTimer（timer fires → 往 tick_tx 有界发送）─────────
            OwnedEffect::ScheduleTimer { id, after_ms } => {
                record_operation(metrics, "schedule_timer", "ok");
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                record_operation(metrics, "cancel_timer", "ok");
                timer_registry.cancel(id);
            }
        }
    }
}

fn record_operation(metrics: &dyn AsyncMetricSink, operation: &'static str, status: &'static str) {
    if !metrics.is_enabled() {
        return;
    }
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::OperationsTotal,
        1.0,
        MetricLabels::one(LabelKey::Stage, "effect_dispatch")
            .with(LabelKey::Operation, operation)
            .with(LabelKey::Status, status),
    ));
}

fn record_error(metrics: &dyn AsyncMetricSink, error_kind: &'static str) {
    if !metrics.is_enabled() {
        return;
    }
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::ErrorsTotal,
        1.0,
        MetricLabels::one(LabelKey::Stage, "effect_dispatch").with(LabelKey::ErrorKind, error_kind),
    ));
}

fn record_wait(
    metrics: &dyn AsyncMetricSink,
    id: MetricId,
    queued_at: Option<Instant>,
    operation: &'static str,
) {
    let Some(queued_at) = queued_at else {
        return;
    };
    let _ = metrics.try_record(MetricEvent::histogram(
        id,
        queued_at.elapsed().as_secs_f64(),
        MetricLabels::one(LabelKey::Stage, "effect_queue").with(LabelKey::Operation, operation),
    ));
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
