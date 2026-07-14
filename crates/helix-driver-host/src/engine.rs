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
//! - `Tr: Transport`      —— `Effect::Send` 出站路由（TransportTable）
//! - `C: Clock`           —— `now_ms` 注入（确定性）
//!
//! ## 五不变量（破坏须同步改 helix-driver-native AGENTS.md / 本头注）
//!
//! 1. 回灌必达：PortReply 走独立 unbounded reply_tx（🔴 绝不改有界）。
//! 2. 主循环不等 I/O：Persist / 必达 Http 入有界队列即返回；满则 Block 传导回 tick_tx
//!    （🔴 抽水绝不放泵循环）。HttpFire 走 DropNewest（满即丢当前条目，泵不阻塞）。
//! 3. 同 key 写顺序：Persist per-table-set N=1 单消费者保序。
//! 4. 回灌优先：biased select reply_rx 在前。
//! 5. graceful drain：返回 = 全部已接收 Persist **已落地** + 必达 Http **已持久化(乐观态)**。
//!    必达 Http drain 有上限（`HTTP_DRAIN_LIMIT`）：在途请求超时则 abort，不等满网络 timeout。
//!    「必达」由乐观态(status=Sending)落库 + 重连重发对账承担，不由「drain 必等满网络」承担——
//!    生命周期（destroy）绝不被不可控的网络 I/O 阻塞（fix/lifecycle-net-decouple）。
//!    Persist drain 仍无上限（本地 DB 写，毫秒级，无网络不可控性，保「同 key 写顺序+全落地」）。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use helix_core::effect::TransportId;
use helix_core::ports::{Clock, FileUploader, HttpRequester, Storage, Transport};
use helix_core::{ExecutionShell, Tick};

pub use crate::batch_sink::BatchSink;
use crate::dispatch::dispatch_effects;
use crate::owned_effect::consume_effects;
use crate::pools::{spawn_http_pool, spawn_upload_pool, PersistJobPayload};
use crate::spawner::{BoundedSpawner, Overflow};
use crate::timer::TimerRegistry;
mod perf_metrics;
mod shutdown;
mod types;
use perf_metrics::EngineMetricRecorder;
use shutdown::drain_pools;

pub use types::{
    EngineDeps, TransportLifecycleEvent, TransportTable, TransportTraceEvent, TransportTraceSink,
    TransportTraceStats, TRANSPORT_TRACE_QUEUE_CAPACITY,
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
/// + 必达 Http 落地后返回。
#[allow(clippy::too_many_arguments)]
pub async fn run_engine_loop<S, H, U, E, Tr, C>(
    mut shell: ExecutionShell,
    mut tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    deps: EngineDeps<S, H, U, E, C>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut transports: TransportTable<Tr>,
    mut transport_rx: mpsc::UnboundedReceiver<(TransportId, Arc<Tr>)>,
) where
    S: Storage + Send + Sync + 'static,
    H: HttpRequester + Send + Sync + 'static,
    U: FileUploader + Send + Sync + 'static,
    E: BatchSink + Send + Sync + 'static,
    // `Send + Sync`：`Arc<Tr>` 经 `transport_rx` 跨后台 connect 任务边界共享。
    Tr: Transport + Send + Sync + 'static,
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
    let http_n = deps.max_http_inflight.max(1);
    let mut timer_registry = TimerRegistry::new();

    // 独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）。
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();

    // per-table-set Persist BoundedSpawner 映射（table_set_key → BoundedSpawner，N=1 保写序）。
    let mut persist_workers: HashMap<String, BoundedSpawner<PersistJobPayload>> = HashMap::new();

    // Http(必达)：N=http_n，Block 溢出（旧 Semaphore 并发闸收敛进 N 个 worker）。
    let http_pool = spawn_http_pool(http_n, Arc::clone(&http), reply_tx.clone(), Overflow::Block);
    let upload_pool = spawn_upload_pool(
        http_n,
        Arc::clone(&uploader),
        reply_tx.clone(),
        Overflow::Block,
    );
    // HttpFire(可丢)：N=http_n，DropNewest 溢出（满即丢当前条目，丢失靠 cursor-gate 重发自愈）。
    // ⚠️ 有意取舍：当前零生产者（IM 三 Http 均必达）；为类型对称 eager 建，N worker park 近零成本。
    // 拆除/重审触发：第二个业务模块接入时（见 native AGENTS.md「跨模块公平性」）。
    let http_fire_pool = spawn_http_pool(
        http_n,
        Arc::clone(&http),
        reply_tx.clone(),
        Overflow::DropNewest,
    );

    // 初始化：触发 on_start。
    let initial_effects = match shell.start() {
        Ok(effects) => consume_effects(effects),
        Err(e) => {
            tracing::error!("helix engine start failed: {}", e);
            return;
        }
    };
    tracing::info!(
        "helix engine started, {} initial effects",
        initial_effects.len()
    );

    // 与主循环同门控：有初始 effect 才 dispatch + flush（避免空批跨 FFI 边界的无谓回调）。
    if !initial_effects.is_empty() {
        dispatch_effects(
            initial_effects,
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
            perf_metrics.sink(),
        )
        .await;
        // start() 那批 Emit 已 dispatch 完成 → 攒批 egress（HX-C007）。native no-op。
        event_sink.flush();
    }

    // 事件泵主循环。
    //
    // biased select!：reply_rx 排在前 → PortReply 优先于新入站 Tick（不变量4）。
    // shutdown_armed：oneshot 只允许被 poll 到完成一次（poll-after-completion 会 panic），
    // 完成后该分支永久解除武装。
    let mut shutdown_armed = true;
    loop {
        perf_metrics.record_queue_snapshot(tick_rx.len(), tick_rx.max_capacity());
        let tick = tokio::select! {
            biased;

            // 优先：PortReply 回灌
            Some(reply) = reply_rx.recv() => reply,

            // 紧随回灌：transport 注册（连接成功后投递的句柄进表）。
            // insert 是 O(1) 内存操作、非 I/O，单独 continue 不进 step（守不变量2「主循环不等
            // I/O」）。排在 reply_rx 之后保「回灌优先」（不变量4）；排在 shutdown/tick 之前确保
            // 连接成功的句柄尽快可见，缩短「连上但句柄未入表」窗口（μs 级）。
            Some((id, tr)) = transport_rx.recv() => {
                tracing::info!(transport_id = id.raw(), "transport 连接成功，注册入路由表");
                transports.insert(id, tr);
                continue;
            }

            // 次选：shutdown 信号（graceful drain 入口）
            res = &mut shutdown_rx, if shutdown_armed => {
                shutdown_armed = false;
                if res.is_ok() { break; }
                continue;
            }

            // 三选：入站 Tick（Command / Inbound / Timer / …）
            tick = tick_rx.recv() => {
                match tick {
                    Some(t) => t,
                    None => break, // tick 通道关闭，进入 graceful drain
                }
            }
        };

        let metric_context = perf_metrics.on_tick(&tick);

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

        let _trace_scope = trace.on_tick_start(&tick);
        let now_ms = clock.now_ms();
        let step_started = std::time::Instant::now();
        let owned = match shell.step(tick, now_ms) {
            Ok(effects) => consume_effects(effects),
            Err(e) => {
                perf_metrics.on_step_error(&metric_context);
                tracing::error!("engine step error: {}", e);
                continue;
            }
        };
        perf_metrics.on_step_complete(&metric_context, step_started.elapsed(), &owned);

        if !owned.is_empty() {
            dispatch_effects(
                owned,
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
                perf_metrics.sink(),
            )
            .await;
            // 本 step 的 Emit 已全部 dispatch → 攒批 egress 一次过边界（HX-C007）。
            // owned 为空时无 Emit、无需 flush（避免空批跨 FFI 边界的无谓回调）。
            event_sink.flush();
            perf_metrics.on_dispatch_complete(&metric_context);
        }
    }

    drain_pools(
        persist_workers,
        upload_pool,
        http_pool,
        http_fire_pool,
        reply_rx,
    )
    .await;

    tracing::info!(
        "helix engine loop exited (persists drained; 必达 upload/http drained or aborted at limit)"
    );
}
