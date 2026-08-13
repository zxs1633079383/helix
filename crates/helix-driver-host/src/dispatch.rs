//! dispatch — 一批 OwnedEffect 的分流兑现（泵核子步骤，从 engine 拆出）。
//!
//! 泵线程绝不在此 await worker 完成（抽水在独立 worker，submit 即返回；死锁红线，不变量2）。
//! Persist/Http 走 BoundedSpawner submit（Block 满时背压传导回 tick_tx）；
//! Send inline await（WS send 快）；Emit/Timer 同步；Request 当前 deferred stub。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use crate::engine::perf_metrics::EngineMetricRecorder;
use crate::engine::TransportTable;
use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};
use crate::owned_effect::OwnedEffect;
use crate::pools::{get_or_spawn_persist, PersistJobPayload};
use crate::spawner::{BoundedSpawner, FeedbackSink, Job};
use crate::table_set_key;
use crate::tick_ingress::EngineTickSender;
use crate::timer::TimerRegistry;
use crate::trace::TraceHooks;
use helix_core::effect::{FileUploadRequest, HttpRequest};
use helix_core::ports::{EventSink, FrameSender, Storage};

/// 把一批 OwnedEffect 分流兑现（async，BoundedSpawner submit，Block 满时等待）。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_effects<S, E, Fs>(
    effects: Vec<OwnedEffect>,
    tick_tx: &EngineTickSender,
    storage: &Arc<S>,
    http_pool: &BoundedSpawner<HttpRequest>,
    upload_pool: &BoundedSpawner<FileUploadRequest>,
    http_fire_pool: &BoundedSpawner<HttpRequest>,
    event_sink: &Arc<E>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &FeedbackSink,
    persist_workers: &mut HashMap<String, BoundedSpawner<PersistJobPayload>>,
    transports: &TransportTable<Fs>,
    trace: &TraceHooks,
    perf_metrics: &mut EngineMetricRecorder,
) where
    S: Storage + Send + Sync + 'static,
    E: EventSink,
    Fs: FrameSender + Send + 'static,
{
    for effect in effects {
        let effect_kind = dispatch_effect_kind(&effect);
        let dispatch_started = perf_metrics.start_timer();
        perf_metrics.on_effect_dispatch(&effect);
        let metrics = perf_metrics.sink();
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
                    perf_metrics.sink_handle(),
                );
                // Block submit：队列满时 .await 阻塞主循环，背压传导到 tick_tx（不变量2）。
                let queued_at = metrics.is_enabled().then(Instant::now);
                sp.submit(Job {
                    corr: Some(corr),
                    payload: PersistJobPayload {
                        ops,
                        atomic: false,
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

            // ── PersistAtomic（带 corr，跨 StorageOp 同一事务）────────────────
            OwnedEffect::PersistAtomic { corr, ops } => {
                trace.on_storage_dispatch(Some(corr), &ops);
                let key = table_set_key(&ops);
                let sp = get_or_spawn_persist(
                    persist_workers,
                    key,
                    Arc::clone(storage),
                    reply_tx.clone(),
                    perf_metrics.sink_handle(),
                );
                let queued_at = metrics.is_enabled().then(Instant::now);
                sp.submit(Job {
                    corr: Some(corr),
                    payload: PersistJobPayload {
                        ops,
                        atomic: true,
                        trace: trace.storage_trace_context(),
                    },
                })
                .await;
                record_wait(
                    metrics,
                    MetricId::PersistQueueWaitSeconds,
                    queued_at,
                    "persist_atomic",
                );
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
                    perf_metrics.sink_handle(),
                );
                let queued_at = metrics.is_enabled().then(Instant::now);
                sp.submit(Job {
                    corr: None,
                    payload: PersistJobPayload {
                        ops,
                        atomic: false,
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

            // ── Send（查路由表 → FrameSender::send 真发 WS）────────────────
            // inline await（WS send 快）。失败/未注册只 warn，绝不让泵崩。
            OwnedEffect::Send {
                transport,
                mut frame,
            } => match transports.get(&transport) {
                Some(t) => {
                    let frame_len = frame.len();
                    let send_started = metrics.is_enabled().then(Instant::now);
                    trace.on_ws_send(transport, &mut frame);
                    // 表内 `Arc<Fs>`（无外层锁）→ 直接 `t.send()`（`&self`）。并发 send 由
                    // sender 内部 state 锁串行化（既有锁，非本路径新增 per-Send 锁，守 HX-C005）。
                    let result = t.send(frame).await;
                    record_ws_send(metrics, frame_len, send_started, result.is_ok());
                    if let Err(e) = result {
                        record_effect_dispatch_error(metrics, "ws_send_failed");
                        tracing::warn!(
                            transport_id = transport.raw(),
                            error = %e,
                            "Effect::Send 发送失败（连接异常，待重连/补偿兜底）"
                        );
                    }
                }
                None => {
                    record_ws_send(metrics, frame.len(), None, false);
                    record_effect_dispatch_error(metrics, "ws_transport_missing");
                    record_operation(metrics, "ws_send", "error");
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
                record_effect_dispatch_error(metrics, "request_deferred");
                record_error(metrics, "request_deferred");
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
                let emit_started = metrics.is_enabled().then(Instant::now);
                event_sink.emit(event);
                if let Some(started) = emit_started {
                    let labels = MetricLabels::one(LabelKey::Stage, "event");
                    let _ = metrics.try_record(MetricEvent::counter(
                        MetricId::EventEmittedTotal,
                        1.0,
                        labels,
                    ));
                    let _ = metrics.try_record(MetricEvent::histogram(
                        MetricId::EventEmitDurationSeconds,
                        started.elapsed().as_secs_f64(),
                        labels,
                    ));
                }
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
        if let Some(started) = dispatch_started {
            let _ = metrics.try_record(MetricEvent::histogram(
                MetricId::EffectDispatchDurationSeconds,
                started.elapsed().as_secs_f64(),
                MetricLabels::one(LabelKey::Stage, "effect")
                    .with(LabelKey::EffectKind, effect_kind),
            ));
        }
    }
}

/// 记录 WS send 的吞吐、字节、耗时与结果，禁止 frame 内容进入标签。
fn record_ws_send(
    metrics: &dyn AsyncMetricSink,
    frame_len: usize,
    started: Option<Instant>,
    success: bool,
) {
    if !metrics.is_enabled() {
        return;
    }
    let labels = MetricLabels::one(LabelKey::Stage, "ws")
        .with(LabelKey::Direction, "outbound")
        .with(LabelKey::Status, if success { "ok" } else { "error" });
    let _ = metrics.try_record(MetricEvent::counter(MetricId::WsFramesTotal, 1.0, labels));
    let _ = metrics.try_record(MetricEvent::histogram(
        MetricId::WsFrameBytes,
        frame_len as f64,
        labels,
    ));
    if let Some(started) = started {
        let _ = metrics.try_record(MetricEvent::histogram(
            MetricId::WsSendDurationSeconds,
            started.elapsed().as_secs_f64(),
            labels,
        ));
    }
    if !success {
        let _ = metrics.try_record(MetricEvent::counter(
            MetricId::WsSendErrorsTotal,
            1.0,
            labels,
        ));
    }
}

/// 记录 Effect 在 host dispatch 边界失败的结构化分母。
fn record_effect_dispatch_error(metrics: &dyn AsyncMetricSink, error_kind: &'static str) {
    if metrics.is_enabled() {
        let _ = metrics.try_record(MetricEvent::counter(
            MetricId::EffectDispatchErrorsTotal,
            1.0,
            MetricLabels::one(LabelKey::Stage, "effect").with(LabelKey::ErrorKind, error_kind),
        ));
    }
}

/// 将 OwnedEffect 映射为冻结的 dispatch 标签，不分配调试字符串。
fn dispatch_effect_kind(effect: &OwnedEffect) -> &'static str {
    match effect {
        OwnedEffect::Persist { .. } => "persist",
        OwnedEffect::PersistAtomic { .. } => "persist_atomic",
        OwnedEffect::PersistFire { .. } => "persist_fire",
        OwnedEffect::Http { .. } => "http",
        OwnedEffect::HttpFire { .. } => "http_fire",
        OwnedEffect::UploadFile { .. } => "upload",
        OwnedEffect::Send { .. } => "send",
        OwnedEffect::Request { .. } => "request",
        OwnedEffect::Emit { .. } => "emit",
        OwnedEffect::ScheduleTimer { .. } => "schedule_timer",
        OwnedEffect::CancelTimer { .. } => "cancel_timer",
    }
}

fn record_operation(metrics: &dyn AsyncMetricSink, operation: &'static str, status: &'static str) {
    if !metrics.is_enabled() {
        return;
    }
    let _ = metrics.try_record(MetricEvent::counter(
        MetricId::OperationsTotal,
        1.0,
        MetricLabels::one(LabelKey::Stage, operation_stage(operation))
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
        MetricLabels::one(LabelKey::Stage, "effect").with(LabelKey::ErrorKind, error_kind),
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
        MetricLabels::one(LabelKey::Stage, operation_stage(operation))
            .with(LabelKey::Operation, operation),
    ));
}

fn operation_stage(operation: &str) -> &'static str {
    match operation {
        "http" | "http_fire" | "upload_file" => "http",
        "persist" | "persist_fire" => "storage",
        "ws_send" => "ws",
        "event_emit" => "event",
        _ => "effect",
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
