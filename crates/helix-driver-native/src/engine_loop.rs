//! engine_loop — tokio 事件泵
//!
//! ## tokio::spawn 在此，不在 helix-core
//!
//! helix-core 保持零运行时依赖；所有异步协调（WS recv、HTTP 响应、timer fired、
//! AppCommand）在此 tokio mpsc 统一入站。
//!
//! ## 架构（E1 修复 + review 回炉后）
//!
//! ```text
//! +- tick_rx (mpsc, cap=256) ─────────────────────────────────────────────────+
//! |  WS reads (per transport) -> Tick::Inbound / Connected / Disconnected      |
//! |  timer spawn             -> Tick::Timer                                     |
//! |  AppCommand (external)   -> Tick::Command                                  |
//! +───────────────────────────────────────────────────────────────────────────+
//!                v  biased select（reply_rx 优先）
//!     ExecutionShell::step(tick, now_ms)
//!                v &[Effect] (borrows shell.scratch, valid until next step)
//!     consume_effects() -> collect to OwnedEffect (frees borrow)
//!     dispatch_owned_effects() -> dispatch by type，立即返回不等磁盘
//!                                  Persist → 入 per-table-set 有序 worker 队列
//!
//! +- reply_tx (unbounded mpsc) ─────────────────────────────────────────────+
//! |  per-table worker tasks  -> Tick::PortReply（写完即回灌，不等引擎收）     |
//! |  http spawn              -> Tick::PortReply                               |
//! +─────────────────────────────────────────────────────────────────────────+
//! ```
//!
//! ## E1 死锁根因与修复
//!
//! 原实现：PortReply 回灌与入站 Tick 共用同一条有界 mpsc(cap=256)，且
//! dispatch_owned_effects 末尾 await 全部 persist handles → 洪峰 >256 环形等待。
//!
//! ## 统一原语 `BoundedSpawner`（刀2-driver）
//!
//! Persist / Http(必达) / HttpFire(可丢) 不再各写一套手写背压骨架，统一成
//! `BoundedSpawner`：**有界 mpsc 队列(cap=K) + N 个常驻 worker task + 溢出策略
//! (Block | DropOldest) + 独立 unbounded reply_tx 回灌**。三类只是实例化参数差异，
//! 纪律集中在一处（Dioxus「统一流」取舍：一个背压/批次/调度策略管全部，纪律不散落）。
//!
//! - **Persist**：per-table-set 懒建，每实例 N=1（保写序），Block 溢出。
//! - **Http（必达）**：全局单例 N=max_http_inflight(默认 8)，Block 溢出。旧 Semaphore
//!   并发闸的语义（permit 数）收敛进 N 个 worker——第 N+1 条请求在有界队列里等 worker，
//!   不在泵里阻塞（Block 模式队未满时 send 立即返回）。
//! - **HttpFire（可丢）**：全局单例 N=8，DropOldest 溢出（背压下丢最旧 + warn，
//!   丢失靠 cursor-gate 重发自愈）。
//!
//! ## 修复五不变量（BoundedSpawner 统一版）
//!
//! 1. **回灌必达**：PortReply 走独立 unbounded channel（reply_tx），永不堵塞 worker。
//!    🔴 **红线**：reply_tx 绝不能改有界——一旦有界且泵唯一消费，Block 模式的死锁环回来
//!    （泵 block 在入队 → 不消费 reply_rx → reply 满 → worker block 在回报 → 不抽队列）。
//! 2. **主循环不等 I/O（Block 传导回源）**：dispatch 把 Persist / 必达 Http 入有界队列后
//!    立即返回（未满时）；满时 `send().await` 阻塞主循环（背压传导回 tick_tx）。
//!    🔴 **红线**：抽水绝不能放泵循环——任何「dispatch 末尾 await worker 完成」都会重新
//!    引入自死锁（泵 block 就抽不了水）。抽水物理上在独立 worker task，submit 后立即返回循环。
//!    HttpFire 走 DropOldest，泵永不阻塞（try_send 满即丢）。
//! 3. **同组写顺序保持**：Persist routing key = ops 涉及的全部 table 名（有序集合，"|" 分隔），
//!    per-table-set worker **强制 N=1**（单消费者保序）。单表 ops → 单表 key；多表 ops →
//!    组合 key（独立 worker，不与单表交叉）。Http/HttpFire 无写序要求（N=8）。
//! 4. **回灌优先**：biased select! 把 reply_rx 排在 tick_rx 前，防 PortReply 饥饿。
//! 5. **退出 drain**：tick_rx 关闭后，shutdown 全部 BoundedSpawner（drop tx → worker
//!    recv None 退出 → join），drain reply_rx，确保「run_engine_loop 返回 = 全部已接收
//!    Persist + 必达 Http 均已落地」。HttpFire 可丢，不纳入「必达落地」保证。
//!
//! ## Block 模式死锁安全证明（与现有 Persist 模型同构）
//!
//! 泵阻塞在「入队 submit().await（满）」是安全的，当且仅当：① 抽水在独立 worker task
//! （不是泵自己 drain）② PortReply 走独立 unbounded reply_tx。依赖链是单向 DAG
//! （泵等 worker，worker 不等泵）：worker `recv → run(I/O) → reply_tx.send(unbounded,不阻塞)
//! → 队列腾位 → 泵 send 解开`。无环 → 无死锁。这正是现有 `get_or_spawn_worker` 已压测验证
//! 的形态，BoundedSpawner 只是把它抽成通用原语复用到 Http，不引入新死锁面。
//!
//! ## Effect 借用收集策略（关键）
//!
//! `ExecutionShell::step()` 返回 `&[Effect]`，借用 `shell.scratch`（跨 step 复用 Vec）。
//! 下次 `step()` 前必须消费完。策略：立即 `consume_effects()` 把 `&[Effect]` 转成
//! `Vec<OwnedEffect>`（owned struct，按需 Clone 字段），脱离借用后再 async 兑现。
//!
//! ## Effect 分流策略
//!
//! - `Persist` / `PersistFire`：按目标表名分组，同组顺序执行（per-table-set BoundedSpawner，N=1）
//! - `Http`（必达）：submit 到 Http BoundedSpawner（N=max_http_inflight，Block 溢出）。
//!   N 个 worker 抢有界队列，泵只入队不发请求——第 N+1 条在队列里等 worker，不挂泵线程。
//!   （旧 Semaphore 限流的并发语义收敛进 N 个 worker；旧注释「受 reqwest 连接池自限」有误：
//!   reqwest pool 不限新连接并发，冷启动 N 条齐发会瞬间打满底层 connect → 全 Network。）
//! - `HttpFire`（可丢）：submit 到 HttpFire BoundedSpawner（N=8，DropOldest 溢出）。
//! - `Send`：tracing 记录（生产接入时调用者传入 Transport map）
//! - `Emit`：同步调用 EventSink::emit
//! - `ScheduleTimer` / `CancelTimer`：TimerRegistry 操作
//!
//! ## rows→bytes 序列化约定
//!
//! `scan` 返回 `Vec<Row>`，driver 统一用 JSON 序列化进 ReplyBytes：
//! `[{"col1": val1, ...}, ...]`，SqlValue 映射：
//!   - Text(s)    → `"s"` (JSON string)
//!   - Integer(i) → `i`  (JSON number)
//!   - Real(f)    → `f`  (JSON number)
//!   - Null       → `null`
//! 反序列化时按 JSON 类型还原 SqlValue（整数 → Integer，浮点 → Real）。

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use serde_json::json;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;

use helix_core::effect::{
    Correlation, DomainEventBytes, Effect, HttpRequest, StorageOp, TimerId, TransportId,
};
use helix_core::ports::{Clock, EventSink, HttpRequester, Transport};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::{classify_port_error, execute_storage_ops, table_set_key};
pub use helix_driver_host::{rows_from_reply_bytes, rows_to_reply_bytes};

use crate::clock::NativeClock;
use crate::event_sink::NativeEventSink;
use crate::http::NativeHttp;
use crate::storage::NativeStorage;
use crate::timer::TimerRegistry;
use crate::transport::NativeTransport;

/// TransportId → 实际 WS 连接 的路由表。
///
/// A1：host 装配阶段填入（`host.add_transport(...)` → `TransportId`）；engine_loop
/// 的 `Effect::Send` 分支按 id 查表调 `NativeTransport::send`。多连接形态从一开始
/// 做成 `HashMap`（非单连接），给第二模块 / 多端口留缝（设计文档 §6 / 02 §4.4）。
/// `Arc<Mutex<_>>`：send 需 `&mut self`，且 host 在装配时也要 `connect()` 同一实例。
pub type TransportTable = HashMap<TransportId, Arc<Mutex<NativeTransport>>>;

/// per-table worker 的 job 队列深度上限（有界背压）。
///
/// 超过此值时 dispatch 侧 `.await` 阻塞主循环，将背压传导到 tick_tx（cap=256）。
/// 这恢复了旧实现"磁盘慢则入站慢"的背压链，同时避免死锁（PortReply 走独立通道）。
const PERSIST_QUEUE_CAP: usize = 1024;

/// Http / HttpFire BoundedSpawner 的有界队列深度上限。
///
/// 必达 Http（Block）满时 `send().await` 把背压传导回 tick_tx；
/// HttpFire（DropOldest）满时 try_send 失败即丢最旧 + warn（自愈靠 cursor-gate 重发）。
const HTTP_QUEUE_CAP: usize = 1024;

/// 事件泵配置（所有依赖项聚合）。
pub struct EngineConfig {
    pub storage: NativeStorage,
    pub http: NativeHttp,
    pub event_sink: NativeEventSink,
    /// Http effect 并发上限（Http BoundedSpawner 的 worker 数 N）。
    ///
    /// 冷启动时 core 一步可吐 N 条 `Effect::Http`（如每 channel 一条 sync/notify），
    /// 若全部齐发会瞬间打满底层连接（reqwest pool **不**限制新连接并发，旧注释假设有误），
    /// 表现为大批 connect 失败 → 全 Network。此字段 = Http/HttpFire BoundedSpawner 的常驻
    /// worker 数（收敛旧 Semaphore permit 语义）：第 N+1 条请求在有界队列里等 worker，不并发齐发。
    /// host（ambient API 合法落点）读 env 注入；建议默认 8。`0` 会被 `.max(1)` 兜底为 1。
    pub max_http_inflight: usize,
}

// ─── OwnedEffect ─────────────────────────────────────────────────────────────
//
// `Effect` 自身不实现 Clone（持有 non-Clone Bytes 和 Vec），
// 但 `bytes::Bytes` 是 O(1) clone（引用计数），其他字段都可 clone。
// 此 owned 镜像立即从 `&[Effect]` 转换，脱离 shell.scratch 借用。

enum OwnedEffect {
    Send {
        transport: TransportId,
        frame: Bytes,
    },
    Persist {
        corr: Correlation,
        ops: Vec<StorageOp>,
    },
    PersistFire {
        ops: Vec<StorageOp>,
    },
    Http {
        corr: Correlation,
        req: HttpRequest,
    },
    HttpFire {
        req: HttpRequest,
    },
    Request {
        corr: Correlation,
        kind: &'static str,
        payload: Bytes,
    },
    Emit {
        event: DomainEventBytes,
    },
    ScheduleTimer {
        id: TimerId,
        after_ms: u64,
    },
    CancelTimer {
        id: TimerId,
    },
}

/// 立即从 `&[Effect]` 转换为 `Vec<OwnedEffect>`（脱离借用）。
///
/// `bytes::Bytes` clone 是 O(1) 引用计数；`Vec` clone 按需复制。
/// 此函数同步执行，在 shell 返回 `&[Effect]` 后立即调用。
fn consume_effects(effects: &[Effect]) -> Vec<OwnedEffect> {
    effects
        .iter()
        .map(|e| match e {
            Effect::Send { transport, frame } => OwnedEffect::Send {
                transport: *transport,
                frame: frame.clone(),
            },
            Effect::Persist { corr, ops } => OwnedEffect::Persist {
                corr: *corr,
                ops: ops.clone(),
            },
            Effect::PersistFire { ops } => OwnedEffect::PersistFire { ops: ops.clone() },
            Effect::Http { corr, req } => OwnedEffect::Http {
                corr: *corr,
                req: req.clone(),
            },
            Effect::HttpFire { req } => OwnedEffect::HttpFire { req: req.clone() },
            Effect::Request {
                corr,
                kind,
                payload,
            } => OwnedEffect::Request {
                corr: *corr,
                kind,
                payload: payload.clone(),
            },
            Effect::Emit { event } => OwnedEffect::Emit {
                event: event.clone(),
            },
            Effect::ScheduleTimer { id, after_ms } => OwnedEffect::ScheduleTimer {
                id: *id,
                after_ms: *after_ms,
            },
            Effect::CancelTimer { id } => OwnedEffect::CancelTimer { id: *id },
        })
        .collect()
}

// ─── BoundedSpawner — driver 内通用有界并发原语（刀2-driver）──────────────────
//
// Persist / Http(必达) / HttpFire(可丢) 统一成此原语：有界 mpsc 队列(cap=K) + N 个
// 常驻 worker task + 溢出策略 + 独立 unbounded reply_tx 回灌。三类只是参数差异。

/// 溢出策略：有界队列满时怎么办（每实例固定一种）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overflow {
    /// 队满 `send().await` 阻塞泵 → 背压传导回 tick_tx（慢而不丢）。Persist / Http(必达) 用。
    Block,
    /// 队满 try_send 失败即丢（warn，泵不阻塞）。HttpFire 用（丢失靠 cursor-gate 重发自愈）。
    DropOldest,
}

/// 一条工作单元：可选回报 corr（None = fire-and-forget，不发 PortReply）+ job 载荷。
struct Job<P> {
    corr: Option<Correlation>,
    payload: P,
}

/// 通用有界并发原语：有界 mpsc(cap=K) + N 个常驻 worker + 溢出策略。
///
/// ## 死锁安全（与现有 Persist 模型同构，已压测验证）
///
/// 抽水在独立 worker task（不是泵自己 drain），PortReply 走独立 unbounded reply_tx，
/// 故 Block 模式泵阻塞在「入队」是安全的：依赖链是单向 DAG（泵等 worker，worker 不等泵）。
/// 🔴 两条红线：① reply_tx 必须 unbounded（不变量1）② 抽水绝不放泵循环（不变量2）。
struct BoundedSpawner<P: Send + 'static> {
    /// 有界 job 发送端（cap = queue_cap）。
    tx: mpsc::Sender<Job<P>>,
    /// N 个 worker task 句柄，shutdown 时 join 确保积压（必达类）全部落地。
    joins: Vec<JoinHandle<()>>,
    /// 溢出策略。
    overflow: Overflow,
}

impl<P: Send + 'static> BoundedSpawner<P> {
    /// 构造：concurrency=N 个 worker；queue_cap=K；run = 每条 job 的 I/O 执行体
    /// （async 闭包，返回 `PortOutcome`，仅在 job.corr=Some 时回报）。
    ///
    /// N 个 worker 共享单队列（竞争 recv）；Persist 用 N=1 保写序。
    fn new<F, Fut>(
        concurrency: usize,
        queue_cap: usize,
        overflow: Overflow,
        reply_tx: mpsc::UnboundedSender<Tick>,
        run: F,
    ) -> Self
    where
        F: Fn(P) -> Fut + Clone + Send + 'static,
        Fut: std::future::Future<Output = PortOutcome> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel::<Job<P>>(queue_cap.max(1));
        // N worker 共享单队列：Arc<Mutex<rx>>，竞争 recv（公平由 tokio Mutex 保证 FIFO-ish）。
        let rx = Arc::new(Mutex::new(rx));
        let joins = (0..concurrency.max(1))
            .map(|_| {
                let rx = Arc::clone(&rx);
                let reply_tx = reply_tx.clone();
                let run = run.clone();
                tokio::spawn(async move {
                    loop {
                        // 独立 worker 持续抽水。**锁有意跨 recv().await 持有**：tokio mpsc
                        // Receiver 是单消费者（recv 要 &mut self），N worker 共享须互斥。空队列时
                        // 仅一个 worker 持锁 park 在 recv，新 job 唤醒它即出块 drop 锁、下一 worker 接力
                        // （经 mutex 串行交接，非瞬时 N 名额）。真并发在锁外的 run().await(下)，故
                        // N 路 I/O 不受队列锁串行化影响——死锁安全见 CLAUDE.md「Block 模式」节。
                        let job = {
                            let mut guard = rx.lock().await;
                            guard.recv().await
                        };
                        let Some(Job { corr, payload }) = job else {
                            break;
                        };
                        let outcome = run(payload).await; // 执行 I/O（不持泵锁、不持队列锁）
                        if let Some(corr) = corr {
                            // unbounded send：不会阻塞，回灌必达（不变量1）。
                            reply_tx.send(Tick::PortReply { corr, outcome }).ok();
                        }
                        // corr=None → fire-and-forget，不回报。
                    }
                })
            })
            .collect();
        Self {
            tx,
            joins,
            overflow,
        }
    }

    /// 入队一条 job。
    /// - Block：`send().await`，满则阻塞泵（背压传导回 tick_tx）。
    /// - DropOldest：try_send，满则丢 + warn（泵不阻塞）。
    async fn submit(&self, job: Job<P>) {
        match self.overflow {
            Overflow::Block => {
                // 队满阻塞——独立 worker 持续抽水会腾位，泵解开（死锁安全见 struct 文档）。
                self.tx.send(job).await.ok();
            }
            Overflow::DropOldest => {
                // 单生产者下「丢最旧」与「丢最新」在自愈场景等价（都靠 cursor-gate 重发补），
                // 取最简的 try_send-Full-drop（丢本条），warn 即可（YAGNI，不引 worker 端 skip）。
                if self.tx.try_send(job).is_err() {
                    tracing::warn!(
                        "HttpFire 队列满，丢弃本条（fire-and-forget 自愈请求，靠 cursor-gate 重发兜底）"
                    );
                }
            }
        }
    }

    /// graceful drain：drop tx → worker recv None 退出 → join 全部。
    async fn shutdown(self) {
        drop(self.tx);
        for j in self.joins {
            let _ = j.await;
        }
    }
}

// ─── Persist 专用：per-table-set 懒建 BoundedSpawner（N=1 保写序）──────────────

/// 启动（或复用）某 table-set 对应的 Persist BoundedSpawner（N=1 单消费者保序）。
///
/// ## 设计不变量（不变量3）
///
/// 同 key 内所有 Persist/PersistFire 严格按入队顺序执行（N=1 → 单 worker 单消费者）。
/// job queue 有界（`PERSIST_QUEUE_CAP`），超限时 caller `.await` 阻塞，背压传导到 tick_tx。
fn get_or_spawn_persist(
    workers: &mut HashMap<String, BoundedSpawner<Vec<StorageOp>>>,
    table_key: String,
    storage: Arc<NativeStorage>,
    reply_tx: mpsc::UnboundedSender<Tick>,
) -> &BoundedSpawner<Vec<StorageOp>> {
    workers.entry(table_key).or_insert_with(|| {
        let storage = Arc::clone(&storage);
        BoundedSpawner::new(
            1, // N=1：单 worker 单消费者保写序（不变量3）
            PERSIST_QUEUE_CAP,
            Overflow::Block,
            reply_tx,
            move |ops: Vec<StorageOp>| {
                let storage = Arc::clone(&storage);
                async move { execute_storage_ops(storage.as_ref(), &ops).await }
            },
        )
    })
}

/// 执行一条 HTTP 请求，返回 `PortOutcome`（Http / HttpFire worker 共用）。
async fn run_http(http: Arc<NativeHttp>, req: HttpRequest) -> PortOutcome {
    match http.request(req).await {
        Ok(resp) => {
            let body_b64 = base64_encode(&resp.body);
            let json_bytes = serde_json::to_vec(&json!({
                "status": resp.status,
                "headers": resp.headers,
                "body": body_b64,
            }))
            .unwrap_or_default();
            PortOutcome::Ok(ReplyBytes(Bytes::from(json_bytes)))
        }
        Err(e) => PortOutcome::Err(classify_port_error(e)),
    }
}

/// 构造 Http(必达) BoundedSpawner：N=max_http_inflight，Block 溢出。
fn spawn_http_pool(
    concurrency: usize,
    http: Arc<NativeHttp>,
    reply_tx: mpsc::UnboundedSender<Tick>,
    overflow: Overflow,
) -> BoundedSpawner<HttpRequest> {
    BoundedSpawner::new(
        concurrency,
        HTTP_QUEUE_CAP,
        overflow,
        reply_tx,
        move |req: HttpRequest| {
            let http = Arc::clone(&http);
            async move { run_http(http, req).await }
        },
    )
}

// ─── engine loop ─────────────────────────────────────────────────────────────

/// 运行 ExecutionShell 的 tokio 事件泵。
///
/// ## 使用方式
///
/// ```rust,ignore
/// let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
/// let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
/// let config = EngineConfig { storage, http, event_sink, max_http_inflight: 8 };
/// tokio::spawn(run_engine_loop(shell, tick_rx, tick_tx.clone(), config, shutdown_rx));
/// // tick_tx 克隆给外部（Tauri command handler / Transport 读任务 / 测试）注入 Tick
/// // shutdown_tx.send(()) 触发 graceful drain + 退出
/// ```
///
/// ## Shutdown 语义
///
/// 发送 `shutdown_tx.send(())` 后，引擎停止接收新 Tick，执行 graceful drain：
/// 等待全部已入队 Persist 落盘后返回。
/// 返回 = 「全部已接收 Persist 均已落盘」。
pub async fn run_engine_loop(
    shell: ExecutionShell,
    tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    shutdown_rx: oneshot::Receiver<()>,
) {
    // 无 transport 的兼容入口：Send 表为空 → Effect::Send 落 warn（回退既有行为）。
    run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx,
        config,
        shutdown_rx,
        TransportTable::new(),
    )
    .await
}

/// 带 transport 路由表的事件泵入口（A1）。
///
/// 与 [`run_engine_loop`] 唯一区别：`transports` 把 `TransportId → NativeTransport`
/// 接进泵，使 `Effect::Send` 真正发到 WS。host 装配时 `connect()` 各 transport 后填表。
pub async fn run_engine_loop_with_transports(
    mut shell: ExecutionShell,
    mut tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    config: EngineConfig,
    mut shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable,
) {
    let clock = NativeClock;
    let storage = Arc::new(config.storage);
    let http = Arc::new(config.http);
    let event_sink = Arc::new(config.event_sink);
    let http_n = config.max_http_inflight.max(1);
    let mut timer_registry = TimerRegistry::new();

    // E1 修复：独立 unbounded 回灌通道（不与入站 tick_rx 共用，永不堵塞写任务）
    let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();

    // per-table-set Persist BoundedSpawner 映射（table_set_key → BoundedSpawner，N=1 保写序）
    let mut persist_workers: HashMap<String, BoundedSpawner<Vec<StorageOp>>> = HashMap::new();

    // Http(必达)：N=http_n，Block 溢出（旧 Semaphore 并发闸收敛进 N 个 worker）。
    let http_pool = spawn_http_pool(http_n, Arc::clone(&http), reply_tx.clone(), Overflow::Block);
    // HttpFire(可丢)：N=http_n，DropOldest 溢出（丢失靠 cursor-gate 重发自愈）。
    // ⚠️ 有意取舍（drift-review R-low 收敛）：当前**零生产者**——helix-im 三个 Http 的
    // PortReply 均 load-bearing（sync 数据/清 inflight 闸/推 cursor），全为必达 `Http`，
    // 无一吐 `HttpFire`。本池为与 `http_pool` **类型对称**而 eager 建（与 core 即使无消费者
    // 也定义 PersistFire 同理——可丢弃性是声明好的类型契约）。N 个 worker park 在 recv 近零成本，
    // 不懒建（懒建 Option 包装是为零成本空闲 task 加复杂度，反 YAGNI）。
    // **拆除/重审触发**：第二个业务模块（presence/notify）接入时——若仍无 HttpFire 生产者则改懒建或删；
    // 若有则本池即就位。见 CLAUDE.md「已知缺口：跨模块公平性」同节登记。
    let http_fire_pool = spawn_http_pool(
        http_n,
        Arc::clone(&http),
        reply_tx.clone(),
        Overflow::DropOldest,
    );

    // 初始化：触发 on_start
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

    // 事件泵主循环
    //
    // biased select!：reply_rx 排在前 → PortReply 优先于新入站 Tick（§4）。
    // 若 reply_rx 有待处理回灌，优先消费，防 PortReply 在洪峰时饥饿。
    //
    // shutdown_armed：oneshot 只允许被 poll 到完成一次（poll-after-completion 会
    // panic），完成后该分支永久解除武装。
    let mut shutdown_armed = true;
    loop {
        let tick = tokio::select! {
            biased;

            // 优先：PortReply 回灌
            Some(reply) = reply_rx.recv() => reply,

            // 次选：shutdown 信号（graceful drain 入口）
            // Ok(()) = 调用方显式 send(()) → 立即进入 graceful drain；
            // Err(_) = sender 被 drop 而未 send → 视为「无 shutdown 信号」继续运行
            //          （此时唯一退出路径是 tick 通道关闭）。
            // shutdown_armed 防 poll-after-completion：tokio oneshot 完成后再 poll
            // 会 panic——若只写 `Ok(()) = ...`，Err 完成时分支不命中、下一轮 select
            // 重新 poll 已完成的 oneshot 即触发 panic。
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
        }
    }

    // ── Graceful drain（§5，BoundedSpawner 统一版）──────────────────────────
    //
    // tick_rx 关闭 → 不再有新 Tick 进来。此时各 BoundedSpawner 可能还有积压 job。
    //
    // 正确关闭顺序（每个 BoundedSpawner.shutdown 内部：drop tx → worker recv None → join）：
    // 1. shutdown 全部 Persist BoundedSpawner（必达落地）
    // 2. shutdown Http(必达) BoundedSpawner（必达落地）
    // 3. shutdown HttpFire BoundedSpawner（尽力 join，但其语义可丢，不纳入「必达落地」保证）
    // 4. drain reply_rx（捡走写完后才回灌的最后一批 PortReply，避免 log 噪音）
    //
    // 保证：run_engine_loop 返回 = 全部已接收 Persist + 必达 Http 均已落地。
    // 破坏此保证的改动需同步更新本文档注释。
    tracing::info!(
        "helix engine loop: tick_rx closed, draining {} persist + http pools",
        persist_workers.len()
    );

    // step 1: shutdown 全部 Persist BoundedSpawner（drop tx → worker 排空 → join）
    for (_, sp) in persist_workers.drain() {
        sp.shutdown().await;
    }

    // step 2 + 3: shutdown Http(必达) + HttpFire 两个 pool
    http_pool.shutdown().await;
    http_fire_pool.shutdown().await;

    // step 4: drain reply_rx（worker 写完最后 job 发出的 PortReply）
    while reply_rx.try_recv().is_ok() {}

    tracing::info!("helix engine loop exited (all persists + 必达 http drained)");
}

/// 把一批 OwnedEffect 分流兑现（async，BoundedSpawner submit，Block 满时等待）。
///
/// ## 要点（BoundedSpawner 统一版）
///
/// - Persist / PersistFire：按 `table_set_key` 路由到 per-table-set Persist BoundedSpawner
///   （懒建，N=1 保写序），`submit` 在队列满时阻塞主循环传导背压（不变量2/3）。
/// - Http（必达）：submit 到 `http_pool`（Block，满则阻塞泵传导背压；绝不 drop，不变量2）。
/// - HttpFire（可丢）：submit 到 `http_fire_pool`（DropOldest，满即丢 + warn，泵不阻塞）。
/// - 泵线程绝不在此 await worker 完成（抽水在独立 worker，submit 即返回；死锁红线，不变量2）。
/// - PortReply 回灌走 reply_tx（unbounded），由 worker 独立发送（不变量1）。
#[allow(clippy::too_many_arguments)]
async fn dispatch_effects(
    effects: Vec<OwnedEffect>,
    tick_tx: &mpsc::Sender<Tick>,
    storage: &Arc<NativeStorage>,
    http_pool: &BoundedSpawner<HttpRequest>,
    http_fire_pool: &BoundedSpawner<HttpRequest>,
    event_sink: &Arc<NativeEventSink>,
    timer_registry: &mut TimerRegistry,
    reply_tx: &mpsc::UnboundedSender<Tick>,
    persist_workers: &mut HashMap<String, BoundedSpawner<Vec<StorageOp>>>,
    transports: &TransportTable,
) {
    for effect in effects {
        match effect {
            // ── Persist（带 corr，写完后通过 reply_tx 回报）────────────────
            OwnedEffect::Persist { corr, ops } => {
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
            //
            // 死锁不变量：泵在此只做 submit（队未满立即返回；满则 Block 阻塞泵，独立 worker
            // 持续抽水后解开——抽水绝不在泵循环，红线见 BoundedSpawner 文档/不变量2）。
            OwnedEffect::Http { corr, req } => {
                http_pool
                    .submit(Job {
                        corr: Some(corr),
                        payload: req,
                    })
                    .await;
            }

            // ── HttpFire（可丢，DropOldest 溢出，无 corr 不回报）────────────
            // 满即丢 + warn（泵不阻塞），丢失靠 cursor-gate 重发自愈。
            OwnedEffect::HttpFire { req } => {
                http_fire_pool
                    .submit(Job {
                        corr: None,
                        payload: req,
                    })
                    .await;
            }

            // ── Send（A1：查路由表 → NativeTransport::send 真发 WS）────────
            // inline await（WS send 快）。失败/未注册只 warn，绝不让泵崩——
            // 上行帧丢失由上层重连 / sync 补偿兜底，不是泵的不变量（保持 §1-§5）。
            OwnedEffect::Send { transport, frame } => match transports.get(&transport) {
                Some(t) => {
                    if let Err(e) = t.lock().await.send(frame).await {
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
            // TODO(方向①收尾): 接入 HostRequester port，把请求转交宿主，
            //   再把宿主回复包成 Tick::PortReply{corr} 经 reply_tx 回灌。
            //   当前为 deferred stub——仅日志、不回灌（生产模块尚未发起 Request；
            //   core 侧闭环已由 helix-core/tests/request_reply_replay.rs 验证）。
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
                event_sink.emit(event);
            }

            // ── ScheduleTimer ──────────────────────────────────────────────
            OwnedEffect::ScheduleTimer { id, after_ms } => {
                // timer fires → 往 tick_tx（有界）发送；timer 回调走入站通道
                timer_registry.schedule(id, after_ms, tick_tx.clone());
            }

            // ── CancelTimer（幂等）────────────────────────────────────────
            OwnedEffect::CancelTimer { id } => {
                timer_registry.cancel(id);
            }
        }
    }
}

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 简单 base64 编码（HTTP body 放进 JSON 时用，避免 binary 污染 JSON string）。
fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::SqlValue;
    use helix_core::tick::PortError as CorePortError;
    use helix_core::PortError;

    #[test]
    fn test_rows_roundtrip() {
        let rows = vec![vec![
            ("id".to_string(), SqlValue::Integer(1)),
            ("name".to_string(), SqlValue::Text("alice".to_string())),
            ("score".to_string(), SqlValue::Real(9.5)),
            ("extra".to_string(), SqlValue::Null),
        ]];
        let bytes = rows_to_reply_bytes(&rows);
        let restored = rows_from_reply_bytes(&bytes).expect("roundtrip");
        assert_eq!(restored.len(), 1);
        let row = &restored[0];
        assert!(matches!(
            row.iter().find(|(k, _)| k == "id").map(|(_, v)| v),
            Some(SqlValue::Integer(1))
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "name").map(|(_, v)| v),
            Some(SqlValue::Text(s)) if s == "alice"
        ));
        assert!(matches!(
            row.iter().find(|(k, _)| k == "extra").map(|(_, v)| v),
            Some(SqlValue::Null)
        ));
    }

    #[test]
    fn test_rows_to_bytes_empty() {
        let bytes = rows_to_reply_bytes(&[]);
        assert_eq!(&bytes[..], b"[]");
    }

    #[test]
    fn test_classify_port_error_timeout() {
        let e = PortError::Transport("timeout: connection timed out".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Timeout);
    }

    #[test]
    fn test_classify_port_error_network() {
        let e = PortError::Transport("network: connection refused".to_string());
        assert_eq!(classify_port_error(e), CorePortError::Network);
    }

    #[test]
    fn test_classify_port_error_storage() {
        let e = PortError::Storage("disk full".to_string());
        assert!(matches!(classify_port_error(e), CorePortError::Storage(_)));
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }

    #[test]
    fn test_table_set_key_empty() {
        assert_eq!(table_set_key(&[]), "");
    }

    #[test]
    fn test_table_set_key_single_table() {
        use helix_core::effect::UpsertSpec;
        let ops = vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
        })];
        assert_eq!(table_set_key(&ops), "messages");
    }

    #[test]
    fn test_table_set_key_multi_table_sorted() {
        use helix_core::effect::{MonotonicUpsertSpec, UpsertSpec};
        // 两个不同 table 的 ops，顺序无所谓，键必须按字典序排
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
            StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                scope_key: "ch:1".to_string(),
                value: 1,
            }),
        ];
        // messages > _helix_monotonic（字典序），键应是 "_helix_monotonic|messages"
        assert_eq!(table_set_key(&ops), "_helix_monotonic|messages");
    }

    #[test]
    fn test_table_set_key_dedup_same_table() {
        use helix_core::effect::UpsertSpec;
        // 同一 table 出现多次，去重后仍是单 key
        let ops = vec![
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![],
                conflict_key: None,
            }),
        ];
        assert_eq!(table_set_key(&ops), "messages");
    }

    // ─── BoundedSpawner 原语测试（刀2-driver）────────────────────────────────
    //
    // 直接喂 run 闭包（不依赖 NativeHttp/NativeStorage），精确测溢出/写序/回灌语义。

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    /// ① HttpFire 满队列 DropOldest：泵不阻塞，溢出条目被丢，已入队的最终执行。
    #[tokio::test]
    async fn test_bounded_spawner_drop_oldest_no_block() {
        let executed = Arc::new(AtomicU64::new(0));
        let gate = Arc::new(tokio::sync::Notify::new());
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let executed_c = Arc::clone(&executed);
        let gate_c = Arc::clone(&gate);
        // N=1，queue_cap=1，worker 在第一条 job 内 park 在 gate 上 → 队列很快填满。
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1,
            1,
            Overflow::DropOldest,
            reply_tx,
            move |_payload: u64| {
                let executed = Arc::clone(&executed_c);
                let gate = Arc::clone(&gate_c);
                async move {
                    gate.notified().await; // 卡住 worker，制造队满
                    executed.fetch_add(1, Ordering::SeqCst);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            },
        );

        // 提交 100 条：DropOldest 下 submit 永不阻塞（worker 卡住，队满即丢）。
        // 用 timeout 证明「泵不阻塞」——若是 Block 会在此卡死超时。
        let submit_all = async {
            for i in 0..100u64 {
                sp.submit(Job {
                    corr: None,
                    payload: i,
                })
                .await;
            }
        };
        tokio::time::timeout(Duration::from_secs(2), submit_all)
            .await
            .expect("DropOldest submit 必须不阻塞泵（未超时）");

        // 放行 worker，等待消化。queue_cap=1 + worker 卡 1 条 → 大量被丢。
        for _ in 0..200 {
            gate.notify_one();
        }
        sp.shutdown().await;

        let done = executed.load(Ordering::SeqCst);
        // 至多消化 worker-in-flight(1) + queue_cap(1) ≈ 2 条，其余 98 条被丢。
        assert!(
            done < 100,
            "DropOldest 必须丢弃溢出条目，实际执行 {done}/100（应远小于 100）"
        );
    }

    /// ② Http 满队列 Block：泵 submit 阻塞，worker 抽水后解开，零丢失（全部执行）。
    #[tokio::test]
    async fn test_bounded_spawner_block_no_loss() {
        let executed = Arc::new(AtomicU64::new(0));
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let executed_c = Arc::clone(&executed);
        // N=1，queue_cap=1，每条 run 慢一点 → submit 第 3 条起必然阻塞等 worker 抽水。
        let sp: BoundedSpawner<u64> =
            BoundedSpawner::new(1, 1, Overflow::Block, reply_tx, move |_payload: u64| {
                let executed = Arc::clone(&executed_c);
                async move {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                    executed.fetch_add(1, Ordering::SeqCst);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            });

        const N: u64 = 50;
        for i in 0..N {
            // Block：满则阻塞，worker 抽水后解开——不死锁（抽水在独立 worker）。
            sp.submit(Job {
                corr: None,
                payload: i,
            })
            .await;
        }
        // shutdown：drop tx → worker 排空积压 → join，必达保证全部落地。
        sp.shutdown().await;

        assert_eq!(
            executed.load(Ordering::SeqCst),
            N,
            "Block 模式必达零丢失：{N} 条必须全部执行"
        );
    }

    /// ③ Persist 写序：N=1 单 worker 单消费者，入队序 == 执行序（不变量3）。
    #[tokio::test]
    async fn test_bounded_spawner_n1_preserves_order() {
        let order = Arc::new(Mutex::new(Vec::<u64>::new()));
        let (reply_tx, _reply_rx) = mpsc::unbounded_channel::<Tick>();

        let order_c = Arc::clone(&order);
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1, // N=1 保序
            8,
            Overflow::Block,
            reply_tx,
            move |payload: u64| {
                let order = Arc::clone(&order_c);
                async move {
                    // 故意让靠前的条目「执行体」更慢，若并发(N>1)会乱序——N=1 必有序。
                    tokio::time::sleep(Duration::from_millis((10 - (payload % 10)) as u64)).await;
                    order.lock().await.push(payload);
                    PortOutcome::Ok(ReplyBytes::default())
                }
            },
        );

        for i in 0..20u64 {
            sp.submit(Job {
                corr: None,
                payload: i,
            })
            .await;
        }
        sp.shutdown().await;

        let recorded = order.lock().await.clone();
        let expected: Vec<u64> = (0..20).collect();
        assert_eq!(
            recorded, expected,
            "N=1 BoundedSpawner 必须保持入队序 == 执行序"
        );
    }

    /// ②附：Block 模式带 corr 回报 → reply_tx 收到 PortReply（回灌必达，不变量1）。
    #[tokio::test]
    async fn test_bounded_spawner_reports_via_reply_tx() {
        let (reply_tx, mut reply_rx) = mpsc::unbounded_channel::<Tick>();
        let sp: BoundedSpawner<u64> = BoundedSpawner::new(
            1,
            8,
            Overflow::Block,
            reply_tx,
            move |_payload: u64| async move { PortOutcome::Ok(ReplyBytes::default()) },
        );

        sp.submit(Job {
            corr: Some(Correlation::from_raw(42)),
            payload: 1,
        })
        .await;
        sp.shutdown().await;

        let reply = reply_rx.try_recv().expect("应收到 PortReply 回灌");
        match reply {
            Tick::PortReply { corr, .. } => assert_eq!(corr.raw(), 42),
            _ => panic!("期望 Tick::PortReply"),
        }
    }

    /// ④ Http / HttpFire core 变体 round-trip：consume_effects 正确映射到 OwnedEffect。
    #[test]
    fn test_consume_effects_http_and_httpfire_roundtrip() {
        let req = || HttpRequest {
            method: "POST".to_string(),
            url: "http://test/x".to_string(),
            headers: vec![],
            body: None,
        };
        let effects = vec![
            Effect::Http {
                corr: Correlation::from_raw(7),
                req: req(),
            },
            Effect::HttpFire { req: req() },
        ];
        let owned = consume_effects(&effects);
        assert_eq!(owned.len(), 2);
        match &owned[0] {
            OwnedEffect::Http { corr, req } => {
                assert_eq!(corr.raw(), 7);
                assert_eq!(req.url, "http://test/x");
            }
            _ => panic!("期望 OwnedEffect::Http"),
        }
        match &owned[1] {
            OwnedEffect::HttpFire { req } => assert_eq!(req.method, "POST"),
            _ => panic!("期望 OwnedEffect::HttpFire"),
        }
    }
}
