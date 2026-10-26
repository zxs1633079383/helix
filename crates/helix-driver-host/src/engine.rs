//! engine — host 泛型事件泵壳（下沉自 native engine_loop）。
//!
//! ## 谁用这个壳
//!
//! native（broadcast）与 FFI（批处理 vtable）共用此泛型壳，统一维护 pump 不变量。
//!
//! ## 泛型边界
//!
//! - `S: Storage`         —— 落库（per-table-set Persist BoundedSpawner，N=1 保写序）
//! - `H: HttpRequester`   —— Http(必达) + HttpFire(可丢) 两池共用
//! - `E: EventSink`       —— **egress 特化点**：native=broadcast / ffi=批处理 vtable
//! - `Fs: FrameSender`    —— `Effect::Send` 出站路由（TransportTable）
//! - `C: Clock`           —— `now_ms` 注入（确定性）
//!
//! ## 五不变量（破坏须同步改 helix-driver-native AGENTS.md / 本头注）
//!
//! 1. 回灌必达：PortReply 走独立 unbounded reply_tx（🔴 绝不改有界）。
//! 2. 主循环不等 I/O：Persist / 必达 Http 入有界队列即返回；满则 Block 传导回 tick_tx
//!    （🔴 抽水绝不放泵循环）。HttpFire 走 DropNewest（满即丢当前条目，泵不阻塞）。
//! 3. 同 key 写顺序：Persist per-table-set N=1 单消费者保序。
//! 4. 回灌优先但不饿死入站：biased select reply_rx 在前；连续 64 条回灌后若入站已排队，
//!    强制消费 1 条入站，再恢复回灌优先。
//! 5. graceful drain：返回 = 全部已接收 Persist **已落地** + 必达 Http **已持久化(乐观态)**。
//!    必达 Http drain 有上限（`HTTP_DRAIN_LIMIT`）：在途请求超时则 abort，不等满网络 timeout。
//!    「必达」由乐观态(status=Sending)落库 + 重连重发对账承担，不由「drain 必等满网络」承担——
//!    生命周期（destroy）绝不被不可控的网络 I/O 阻塞（fix/lifecycle-net-decouple）。
//!    Persist drain 仍无上限（本地 DB 写，毫秒级，无网络不可控性，保「同 key 写顺序+全落地」）。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use helix_core::ports::{Clock, FileUploader, FrameSender, HttpRequester, Storage};
use helix_core::{ExecutionShell, Tick};

pub use crate::batch_sink::BatchSink;
use crate::dispatch::dispatch_effects_with_context;
use crate::lifecycle::{
    LifecycleCapability, LifecycleContext, LifecycleStage, LifecycleStatus, LifecycleTracker,
};
use crate::owned_effect::{consume_effect_batch, OwnedEffect};
use crate::pools::{spawn_http_pool_observed, spawn_upload_pool_observed, PersistJobPayload};
use crate::spawner::{feedback_channel, BoundedSpawner, FeedbackSink, Overflow};
use crate::tick_ingress::{EngineTickReceiver, EngineTickSender};
use crate::timer::TimerRegistry;
mod entrypoints;
pub(crate) mod perf_metrics;
mod rss;
mod shutdown;
mod types;
use perf_metrics::EngineMetricRecorder;
use shutdown::drain_pools;

const MAX_CONSECUTIVE_REPLIES: usize = 64;

pub use entrypoints::{run_engine_loop, run_engine_loop_stamped};

pub use types::{
    register_transport, EngineDeps, TransportLifecycleEvent, TransportRegistration, TransportTable,
    TransportTraceEvent, TransportTraceSink, TransportTraceStats, TRANSPORT_TRACE_QUEUE_CAPACITY,
};

/// 运行 ExecutionShell 的 tokio 事件泵（泛型壳）。
///
/// ## Transport 注册（连接期句柄类型层不可见 / 消除锁跨 await）
///
/// `transports` 是 loop 私有 `mut` 表（dispatch 只读，不再外层 `Arc<Mutex>`）。装配端有两条
/// 路径填表：① 预填（host-cli 启动前同步 connect 好直接放入，CLI 可阻塞）；② 注册通道
/// （FFI：create 不能 await connect，否则卡满 `DEFAULT_TIMEOUT_SECS`——见 fix/lifecycle-net-decouple）。
/// 路径②下，装配端 spawn 后台任务对**独占 move 进去的裸句柄**跑 `connect().await`，连成功后经
/// `transport_rx` 投递 `(id, Arc<Tr>)`，本 loop 的 biased select 第二臂收到即 `insert`。
///
/// **死锁/停顿安全（类型层论证，非时序不变量）**：连接在途时句柄**根本不在 `transports` 表内**，
/// `Effect::Send` 的 `transports.get()` 返回 `None` → 走既有 warn 兜底；连接期间不存在任何「会被
/// Send 路径争用、且被某任务持有跨 `connect().await` 的锁」（表内 `Arc<Tr>` 无外层锁，并发 send
/// 由 transport 内部 state 锁串行化，那把锁本就存在、非新增）。`transport_rx` 是 unbounded（同
/// `reply_tx` 哲学）→ 后台 connect 任务投递句柄永不阻塞 → 依赖链单向 DAG（loop 等 channel，
/// channel 不等 loop）无环。
///
/// ## Shutdown 语义
///
/// `shutdown_rx` 收到 `()` 或 `tick_rx` 关闭 → graceful drain：等全部已接收 Persist
/// 落地；Http/UploadFile 在上限内 drain，超时则 abort；最后 flush drain 期间
/// 已排队的 driver 诊断，再返回（不保证远端 HTTP 全部成功）。
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_engine_loop_inner<S, H, U, E, Tr, C>(
    mut shell: ExecutionShell,
    mut tick_rx: EngineTickReceiver,
    tick_tx: EngineTickSender,
    deps: EngineDeps<S, H, U, E, C>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut transports: TransportTable<Tr>,
    mut transport_rx: mpsc::UnboundedReceiver<TransportRegistration<Tr>>,
) where
    S: Storage + Send + Sync + 'static,
    H: HttpRequester + Send + Sync + 'static,
    U: FileUploader + Send + Sync + 'static,
    E: BatchSink + Send + Sync + 'static,
    Tr: FrameSender + Send + Sync + 'static,
    C: Clock,
{
    let transport_lifecycle_tx = deps.transport_lifecycle_tx.clone();
    let transport_trace_tx = deps.transport_trace_tx.clone();
    let clock = deps.clock;
    let storage = deps.storage;
    let http = deps.http;
    let uploader = deps.uploader;
    let event_sink = deps.event_sink;
    let trace = deps.trace;
    let mut perf_metrics = EngineMetricRecorder::new(deps.metrics);
    perf_metrics.on_engine_start();
    let rss_sampler = perf_metrics.spawn_rss_sampler();
    let http_n = deps.max_http_inflight.max(1);
    let mut timer_registry = TimerRegistry::with_metrics(perf_metrics.sink_handle());
    let lifecycle_tracker = LifecycleTracker::default();

    // 独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）。
    let (reply_tx, mut reply_rx) = feedback_channel();
    let feedback_sink = FeedbackSink::Stamped(reply_tx.clone());

    // per-table-set Persist BoundedSpawner 映射（table_set_key → BoundedSpawner，N=1 保写序）。
    let mut persist_workers: HashMap<String, BoundedSpawner<PersistJobPayload>> = HashMap::new();

    // Http(必达)：N=http_n，Block 溢出（旧 Semaphore 并发闸收敛进 N 个 worker）。
    let http_pool = spawn_http_pool_observed(
        http_n,
        Arc::clone(&http),
        reply_tx.clone(),
        Overflow::Block,
        perf_metrics.sink_handle(),
    );
    let upload_pool = spawn_upload_pool_observed(
        http_n,
        Arc::clone(&uploader),
        reply_tx.clone(),
        Overflow::Block,
        perf_metrics.sink_handle(),
    );
    // HttpFire(可丢)：N=http_n，DropNewest 溢出（满即丢当前条目，丢失靠 cursor-gate 重发自愈）。
    // ⚠️ 有意取舍：当前零生产者（IM 三 Http 均必达）；为类型对称 eager 建，N worker park 近零成本。
    // 拆除/重审触发：第二个业务模块接入时（见 native AGENTS.md「跨模块公平性」）。
    let http_fire_pool = spawn_http_pool_observed(
        http_n,
        Arc::clone(&http),
        reply_tx.clone(),
        Overflow::DropNewest,
        perf_metrics.sink_handle(),
    );

    // 初始化：触发 on_start。
    let initial_batch = match shell.start() {
        Ok(effects) => consume_effect_batch(effects),
        Err(e) => {
            perf_metrics.on_engine_stopped();
            if let Some(rss_sampler) = rss_sampler {
                rss_sampler.abort();
            }
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    };
    tracing::info!(
        "helix engine started, {} initial effects",
        initial_batch.effects.len()
    );

    // Dispatch remains effect-gated; flush also delivers driver diagnostics on empty steps.
    if !initial_batch.effects.is_empty() {
        let initial_context = LifecycleContext::new(0, None, None)
            .with_capability(LifecycleCapability::Http, true)
            .with_capability(LifecycleCapability::Persist, true)
            .with_capability(LifecycleCapability::Effect, true)
            .with_capability(LifecycleCapability::Ws, !transports.is_empty());
        emit_missing_lifecycle_capabilities(&trace, &initial_context, &initial_batch.effects);
        dispatch_effects_with_context(
            initial_batch.effects,
            &tick_tx,
            &storage,
            &http_pool,
            &upload_pool,
            &http_fire_pool,
            &event_sink,
            &mut timer_registry,
            &feedback_sink,
            &mut persist_workers,
            &transports,
            &trace,
            &mut perf_metrics,
            &initial_context,
            Some(&lifecycle_tracker),
        )
        .await;
    }
    // Sink suppresses empty callbacks; initial diagnostics share this one batch boundary.
    event_sink.flush();

    // 事件泵主循环。
    //
    // biased select!：reply_rx 排在前 → PortReply 优先于新入站 Tick（不变量4）。
    // 有界公平闸避免持续 sync PortReply 洪峰让用户 Command / 新 WS 帧永久排队。
    // shutdown_armed：oneshot 只允许被 poll 到完成一次（poll-after-completion 会 panic），
    // 完成后该分支永久解除武装。
    let mut shutdown_armed = true;
    let mut consecutive_replies = 0usize;
    loop {
        let iteration_started = perf_metrics.start_timer();
        perf_metrics.record_queue_snapshot(tick_rx.len(), tick_rx.max_capacity());
        let forced_ingress = if consecutive_replies >= MAX_CONSECUTIVE_REPLIES {
            let forced = tick_rx.try_recv().ok();
            if forced.is_some() {
                perf_metrics.on_reply_fairness_forced();
            }
            forced
        } else {
            None
        };
        let (tick, queue_wait, ingress_carrier) = if let Some(queued) = forced_ingress {
            queued
        } else {
            tokio::select! {
                biased;

                // 优先：PortReply 回灌
                Some(feedback) = reply_rx.recv() => {
                    let wait = feedback.enqueued_at.elapsed();
                    perf_metrics.on_feedback_dequeued(&feedback.tick, wait, reply_rx.len());
                    (feedback.tick, None, None)
                },

                // 紧随回灌：transport 注册（连接成功后投递的句柄进表）。
                // insert 是 O(1) 内存操作、非 I/O，单独 continue 不进 step（守不变量2「主循环不等
                // I/O」）。排在 reply_rx 之后保「回灌优先」（不变量4）；排在 shutdown/tick 之前确保
                // 连接成功的句柄尽快可见，缩短「连上但句柄未入表」窗口（μs 级）。
                Some(registration) = transport_rx.recv() => {
                    let TransportRegistration { id, sender, registered_tx } = registration;
                    tracing::info!(transport_id = id.raw(), "transport 连接成功，注册入路由表");
                    transports.insert(id, sender);
                    registered_tx.send(()).ok();
                    continue;
                }

                // 次选：shutdown 信号（graceful drain 入口）
                res = &mut shutdown_rx, if shutdown_armed => {
                    shutdown_armed = false;
                    if res.is_ok() { break; }
                    continue;
                }

                // 三选：入站 Tick（Command / Inbound / Timer / …）
                queued = tick_rx.recv() => {
                    match queued {
                        Some(t) => t,
                        None => break, // tick 通道关闭，进入 graceful drain
                    }
                }
            }
        };

        if matches!(tick, Tick::PortReply { .. } | Tick::PortProgress { .. }) {
            consecutive_replies = consecutive_replies.saturating_add(1);
        } else {
            consecutive_replies = 0;
        }

        let metric_context = perf_metrics.on_tick(&tick, queue_wait);

        // A PortReply closes an already-correlated storage/HTTP hop.  Keep only
        // the correlation and result class here: reply bytes may contain a VM
        // or authority payload and must never enter a generic driver log.
        if let Tick::PortReply { corr, outcome } = &tick {
            let outcome_class = if matches!(outcome, helix_core::tick::PortOutcome::Ok(_)) {
                "ok"
            } else {
                "err"
            };
            tracing::debug!(
                hop = "host.port_reply",
                corr = corr.raw(),
                outcome = outcome_class,
                "correlated port reply returned to the deterministic shell"
            );
        }
        if let Tick::PortProgress { corr, progress } = &tick {
            tracing::debug!(
                hop = "host.port_progress",
                corr = corr.raw(),
                completed_bytes = progress.completed_bytes,
                total_bytes = progress.total_bytes,
                "correlated upload progress returned to the deterministic shell"
            );
        }

        if let Tick::Disconnected(transport_id) = &tick {
            if let Some(tx) = &transport_lifecycle_tx {
                tx.send(TransportLifecycleEvent::Disconnected {
                    transport_id: *transport_id,
                    reason: "reader_closed_or_error",
                })
                .ok();
            }
            if let Some(tx) = &transport_trace_tx {
                tx.try_emit(TransportTraceEvent {
                    transport_id: *transport_id,
                    name: "helix.ws.disconnect",
                    action: "disconnect",
                    attempt: None,
                    delay_ms: None,
                    next_delay_ms: None,
                    reason: Some("reader_closed_or_error"),
                    error_class: None,
                });
            }
        }

        let tick_id = lifecycle_tracker.next_tick_id();
        let (parent_tick_id, inherited_carrier, inherited_span_parent) =
            lifecycle_tracker.parent_for_tick(&tick);
        let context = trace
            .context_for_tick(
                &tick,
                tick_id,
                parent_tick_id,
                ingress_carrier.or(inherited_carrier),
            )
            .with_capability(LifecycleCapability::Http, true)
            .with_capability(LifecycleCapability::Persist, true)
            .with_capability(LifecycleCapability::Effect, true)
            .with_capability(LifecycleCapability::Ws, !transports.is_empty());
        let context = context.with_span_parent(inherited_span_parent);
        let (mut trace_scope, context) = trace.start_tick_with_context(&tick, &context);
        let now_ms = clock.now_ms();
        let step_started = perf_metrics.start_timer();
        let batch = match trace_scope.step(&mut shell, tick, now_ms) {
            Ok(effects) => consume_effect_batch(effects),
            Err(e) => {
                perf_metrics.on_step_error(&metric_context);
                perf_metrics.on_loop_iteration(iteration_started);
                tracing::error!("engine step error: {}", e);
                // A rejected core tick can still be the next boundary for late driver facts.
                event_sink.flush();
                continue;
            }
        };
        let context = context.with_capability(
            LifecycleCapability::Ws,
            context.has_capability(LifecycleCapability::Ws)
                || batch
                    .effects
                    .iter()
                    .any(|effect| matches!(effect, OwnedEffect::Send { .. })),
        );
        emit_missing_lifecycle_capabilities(&trace, &context, &batch.effects);
        perf_metrics.on_step_complete(&metric_context, step_started, batch.summary);

        if !batch.effects.is_empty() {
            dispatch_effects_with_context(
                batch.effects,
                &tick_tx,
                &storage,
                &http_pool,
                &upload_pool,
                &http_fire_pool,
                &event_sink,
                &mut timer_registry,
                &feedback_sink,
                &mut persist_workers,
                &transports,
                &trace,
                &mut perf_metrics,
                &context,
                Some(&lifecycle_tracker),
            )
            .await;
        }
        // Late driver diagnostics may exist even when this step emits no business effects.
        event_sink.flush();
        perf_metrics.on_dispatch_complete(&metric_context);
        perf_metrics.on_loop_iteration(iteration_started);
    }

    let shutdown_started = perf_metrics.start_timer();
    drain_pools(
        persist_workers,
        upload_pool,
        http_pool,
        http_fire_pool,
        reply_rx,
    )
    .await;
    // Uploads completed during drain enqueue diagnostics after the last step boundary.
    event_sink.flush();
    perf_metrics.on_shutdown(shutdown_started);
    perf_metrics.on_engine_stopped();
    if let Some(rss_sampler) = rss_sampler {
        rss_sampler.abort();
    }

    tracing::info!(
        "helix engine loop exited (persists drained; 必达 upload/http drained or aborted at limit)"
    );
}

/// 为本 Tick 没有实际执行的能力槽位写入 not_applicable，不创建伪造网络 Span。
fn emit_missing_lifecycle_capabilities(
    trace: &crate::trace::TraceHooks,
    context: &LifecycleContext,
    effects: &[OwnedEffect],
) {
    let has_http = effects.iter().any(|effect| {
        matches!(
            effect,
            OwnedEffect::Http { .. }
                | OwnedEffect::HttpFire { .. }
                | OwnedEffect::UploadFile { .. }
        )
    });
    let has_ws = effects
        .iter()
        .any(|effect| matches!(effect, OwnedEffect::Send { .. }));
    let has_persist = effects.iter().any(|effect| {
        matches!(
            effect,
            OwnedEffect::Persist { .. }
                | OwnedEffect::PersistAtomic { .. }
                | OwnedEffect::PersistFire { .. }
        )
    });
    let has_event = effects
        .iter()
        .any(|effect| matches!(effect, OwnedEffect::Emit { .. }));

    if !has_http {
        trace.emit_lifecycle_status_with_reason(
            context,
            LifecycleStage::T2,
            Some(LifecycleCapability::Http),
            LifecycleStatus::NotApplicable,
            Some("http_effect_absent"),
        );
    }
    if !has_ws {
        trace.emit_lifecycle_status_with_reason(
            context,
            LifecycleStage::T3,
            Some(LifecycleCapability::Ws),
            LifecycleStatus::NotApplicable,
            Some("ws_effect_absent"),
        );
    }
    if !has_persist {
        trace.emit_lifecycle_status_with_reason(
            context,
            LifecycleStage::T4,
            Some(LifecycleCapability::Persist),
            LifecycleStatus::NotApplicable,
            Some("persist_effect_absent"),
        );
    }
    if !has_event {
        trace.emit_lifecycle_status_with_reason(
            context,
            LifecycleStage::T5,
            Some(LifecycleCapability::Effect),
            LifecycleStatus::NotApplicable,
            Some("event_effect_absent"),
        );
    }
}
