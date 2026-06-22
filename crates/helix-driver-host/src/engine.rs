//! engine — host 泛型事件泵壳（下沉自 native engine_loop）。
//!
//! ## 谁用这个壳
//!
//! native（broadcast EventSink）与（下一轮）FFI（批处理 vtable sink）**共用**此泛型壳，
//! 各自只差 EventSink egress 特化 + tick 源接线。pump 核（背压/分流/drain/死锁不变量）
//! 在此一处维护，不再 per-driver 各写一套。
//!
//! ## 泛型边界
//!
//! - `S: Storage`         —— 落库（per-table-set Persist BoundedSpawner，N=1 保写序）
//! - `H: HttpRequester`   —— Http(必达) + HttpFire(可丢) 两池共用
//! - `E: EventSink`       —— **egress 特化点**：native=broadcast / ffi=批处理 vtable
//! - `Tr: Transport`      —— `Effect::Send` 出站路由（TransportTable）
//! - `C: Clock`           —— `now_ms` 注入（确定性）
//!
//! ## 五不变量（破坏须同步改 helix-driver-native CLAUDE.md / 本头注）
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

use tokio::sync::{mpsc, oneshot, Mutex};

use helix_core::effect::{StorageOp, TransportId};
use helix_core::ports::{Clock, EventSink, HttpRequester, Storage, Transport};
use helix_core::{ExecutionShell, Tick};

/// 每 step 批处理钩子（host 层扩展，不污染 core 的 [`EventSink`]）。
///
/// ## 为什么钩子在 host 不在 core
///
/// core 的 `EventSink::emit` 是单条同步「登记意图」契约（sans-IO，HX-C001：core 零运行时）。
/// 「一个 step 产出的 Emit 整批跨边界一次过」是**驱动侧 egress 批处理**（HX-C007），属 driver
/// 关注点。把 flush 钩子放 core 会让 core 认识「批边界」这个运行时概念 → 越界。故在此 host 层
/// 用 supertrait 扩展：pump 在**每次 `dispatch_effects` 之后**调一次 `flush()`，让需要批 egress
/// 的 sink（FFI vtable）攒批后一次性过 FFI；不需要的（native broadcast 即时发）用默认 no-op。
///
/// ## 不变量
///
/// - `flush()` 调用点 = 每个 step 的 effects 全部 dispatch 完成之后（含初始 `start()` 那批）。
/// - 默认实现 no-op：native `impl BatchSink for NativeEventSink {}` 行为零变。
/// - flush 内不得 panic（FFI 实现跨边界回调，遵 HX-C006：生产 cdylib panic=abort）。
pub trait BatchSink: EventSink {
    /// 一个 step 的全部 Emit 已 dispatch 完成 → 把本 step 攒下的批整体 egress。
    ///
    /// 默认 no-op（即时发的 sink 无需批，如 native broadcast）。
    fn flush(&self) {}
}

use crate::dispatch::dispatch_effects;
use crate::owned_effect::consume_effects;
use crate::pools::spawn_http_pool;
use crate::spawner::{BoundedSpawner, Overflow};
use crate::timer::TimerRegistry;

/// graceful drain 时「必达 Http」在途请求的 join 上限（fix/lifecycle-net-decouple）。
///
/// 必达 Http reqwest 单条可卡满 `DEFAULT_TIMEOUT_SECS`（连不上 host ~30s）。drain 给此上限：
/// 超时 abort 在途请求，让生命周期（helix_destroy）快速退出，而非等满网络 timeout。
/// 不丢保证由「乐观态持久化 + 重连重发对账」承担（见五不变量⑤ + `shutdown_with_timeout` doc）。
/// 取 3s：足够覆盖正常 LAN 内 POST /posts/create 往返（毫秒级），又远小于 30s 网络 timeout。
const HTTP_DRAIN_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

/// `TransportId → 实际连接` 路由表（泛型 over Transport 实现）。
///
/// host 装配阶段填入；`Effect::Send` 分支按 id 查表调 `Transport::send`。
/// `Arc<Mutex<_>>`：send 需 `&mut self`，且 host 装配时也要 `connect()` 同一实例。
pub type TransportTable<Tr> = HashMap<TransportId, Arc<Mutex<Tr>>>;

/// 事件泵依赖聚合（泛型 ports + 配置）。
///
/// native / ffi 各自构造此结构（注入自家 EventSink egress），调 [`run_engine_loop`]。
pub struct EngineDeps<S, H, E, C> {
    pub storage: Arc<S>,
    pub http: Arc<H>,
    pub event_sink: Arc<E>,
    pub clock: C,
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。`0` → `.max(1)` 兜底 1。
    pub max_http_inflight: usize,
}

/// 运行 ExecutionShell 的 tokio 事件泵（泛型壳）。
///
/// ## Shutdown 语义
///
/// `shutdown_rx` 收到 `()` 或 `tick_rx` 关闭 → graceful drain：等全部已接收 Persist
/// + 必达 Http 落地后返回。
#[allow(clippy::too_many_arguments)]
pub async fn run_engine_loop<S, H, E, Tr, C>(
    mut shell: ExecutionShell,
    mut tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    deps: EngineDeps<S, H, E, C>,
    mut shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable<Tr>,
) where
    S: Storage + Send + Sync + 'static,
    H: HttpRequester + Send + Sync + 'static,
    E: BatchSink + Send + Sync + 'static,
    Tr: Transport + Send + 'static,
    C: Clock,
{
    let clock = deps.clock;
    let storage = deps.storage;
    let http = deps.http;
    let event_sink = deps.event_sink;
    let http_n = deps.max_http_inflight.max(1);
    let mut timer_registry = TimerRegistry::new();

    // 独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）。
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();

    // per-table-set Persist BoundedSpawner 映射（table_set_key → BoundedSpawner，N=1 保写序）。
    let mut persist_workers: HashMap<String, BoundedSpawner<Vec<StorageOp>>> = HashMap::new();

    // Http(必达)：N=http_n，Block 溢出（旧 Semaphore 并发闸收敛进 N 个 worker）。
    let http_pool = spawn_http_pool(http_n, Arc::clone(&http), reply_tx.clone(), Overflow::Block);
    // HttpFire(可丢)：N=http_n，DropNewest 溢出（满即丢当前条目，丢失靠 cursor-gate 重发自愈）。
    // ⚠️ 有意取舍：当前零生产者（IM 三 Http 均必达）；为类型对称 eager 建，N worker park 近零成本。
    // 拆除/重审触发：第二个业务模块接入时（见 native CLAUDE.md「跨模块公平性」）。
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
            &http_fire_pool,
            &event_sink,
            &mut timer_registry,
            &reply_tx,
            &mut persist_workers,
            &transports,
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
        let tick = tokio::select! {
            biased;

            // 优先：PortReply 回灌
            Some(reply) = reply_rx.recv() => reply,

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

        let now_ms = clock.now_ms();
        let owned = match shell.step(tick, now_ms) {
            Ok(effects) => consume_effects(effects),
            Err(e) => {
                tracing::error!("engine step error: {}", e);
                continue;
            }
        };

        if !owned.is_empty() {
            dispatch_effects(
                owned,
                &tick_tx,
                &storage,
                &http_pool,
                &http_fire_pool,
                &event_sink,
                &mut timer_registry,
                &reply_tx,
                &mut persist_workers,
                &transports,
            )
            .await;
            // 本 step 的 Emit 已全部 dispatch → 攒批 egress 一次过边界（HX-C007）。
            // owned 为空时无 Emit、无需 flush（避免空批跨 FFI 边界的无谓回调）。
            event_sink.flush();
        }
    }

    // ── Graceful drain（不变量5）──────────────────────────────────────────
    //
    // 正确关闭顺序（每个 BoundedSpawner.shutdown 内部：drop tx → worker recv None → join）：
    // 1. shutdown 全部 Persist（必达落地——本地 DB 写，无网络，无上限 join）
    // 2. shutdown Http(必达)（**有上限** drain：不被在途网络阻塞，超时 abort 在途 reqwest）
    // 3. shutdown HttpFire（尽力 join，可丢，不纳入「必达落地」保证；同样有上限）
    // 4. drain reply_rx（捡走写完后才回灌的最后一批 PortReply）
    tracing::info!(
        "helix engine loop: tick_rx closed, draining {} persist + http pools",
        persist_workers.len()
    );

    // Persist：本地 SQLite 写，毫秒级，无网络不可控性 → 无上限 join（保「同 key 写顺序+全落地」）。
    for (_, sp) in persist_workers.drain() {
        sp.shutdown().await;
    }
    // 必达 Http：给上限（HTTP_DRAIN_LIMIT）。在途 reqwest 连不上 host 时单条卡满
    // DEFAULT_TIMEOUT_SECS（~30s），无上限会让 graceful drain（→ helix_destroy）等满该 timeout。
    // 超时则 abort 在途——乐观态 status=Sending 已落库（真源），不丢，靠重连重发对账兜底
    // （不变量5 改写：见本文件头注「五不变量⑤」+ helix-driver-{host,native} CLAUDE.md）。
    if !http_pool.shutdown_with_timeout(HTTP_DRAIN_LIMIT).await {
        tracing::warn!(
            limit_ms = HTTP_DRAIN_LIMIT.as_millis(),
            "必达 Http drain 超上限——abort 在途请求（乐观态已持久化，靠重连重发对账兜底）"
        );
    }
    // HttpFire：可丢，同样封顶（不阻塞退出）。
    let _ = http_fire_pool.shutdown_with_timeout(HTTP_DRAIN_LIMIT).await;
    while reply_rx.try_recv().is_ok() {}

    tracing::info!("helix engine loop exited (persists drained; 必达 http drained or aborted at limit)");
}
