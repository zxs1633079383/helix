//! pump — FFI 专用 pump 线程装配（复用 host `run_engine_loop` 泛型核）。
//!
//! ## 线程 + 运行时模型（GOAL §1.3：core 单线程跑）
//!
//! `helix_create_with_network` 起一个 `std::thread`，线程内建 **current_thread** tokio runtime
//! （`.enable_all()`——漏则 `spawn_blocking` 内 await 的 timer/IO park 异常），`block_on` host 的
//! `run_engine_loop`。core 在此专属单线程跑（不进多线程 = 与 native multi-thread 的差异点，
//! 但共用同一泛型 pump 核，只换 EventSink egress + Clock + 线程模型）。
//!
//! ## tick 源（grounding E：Rust 持 WS）
//!
//! `SharedWsClient::with_inbound_tick(id, tick_tx)`：入站帧由 Rust 内部直发 `Tick::Inbound`，
//! 不经 C++、不经已删的 `helix_on_ws_frame`。命令经 `helix_command → tick_tx`（api.rs）。
//! Effect::Send/Http/Persist/Timer 全走 host dispatch（已下沉）。

use std::sync::Arc;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::{ExecutionShell, PortError};
use helix_driver_host::engine::{
    self, register_transport, EngineDeps, TransportRegistration, TransportTable,
};
#[cfg(test)]
use helix_driver_host::TraceHooks;
use helix_driver_host::{
    spawn_reconnect_supervisor, AsyncMetricSink, AuthTokenRegistry, BoundedMetricRuntime,
    CommandTraceQueue, HostHeaderRegistry, HostNetworkConfig, HostOtelRuntime, HostStorage,
    ReconnectPolicy, ReconnectTraceSink, SharedFileUploader, SharedHttpClient, SharedWsClient,
    TickIngressReceiver, TickIngressSender,
};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::ImModule;
use tokio::sync::{mpsc, oneshot, watch};

use crate::clock::FfiClock;
use crate::event_sink::FfiEventSink;
use crate::http::FfiHttpRequester;
use crate::vtable::CppHostVtable;

mod observability;
use observability::{ffi_trace_hooks, spawn_transport_trace_exporter};

/// 主 WS TransportId（host 集中分配；FFI 单连接固定 0，对齐 host-cli 约定）。
const MAIN_TRANSPORT: u32 = 0;

/// pump 线程入口参数（create 在主线程构造，move 进线程）。
pub struct PumpArgs {
    pub config: HostNetworkConfig,
    pub headers: HostHeaderRegistry,
    pub auth: AuthTokenRegistry,
    pub command_traces: CommandTraceQueue,
    #[cfg(test)]
    pub trace_override: Option<TraceHooks>,
    pub vtable: CppHostVtable,
    pub tick_tx: TickIngressSender,
    pub tick_rx: TickIngressReceiver,
    pub shutdown_rx: oneshot::Receiver<()>,
    /// 装配成功后只发布 readiness；runtime/storage 不跨回 C ABI 形成同步业务查询旁路。
    pub ready_tx: std::sync::mpsc::SyncSender<Result<(), ()>>,
    pub db_url: String,
    pub max_http_inflight: usize,
    pub metrics: Arc<dyn AsyncMetricSink>,
    pub metric_runtime: Option<BoundedMetricRuntime>,
}

/// FFI 初连与重连的唯一成功路径：握手后先注册 sender，收到 engine ack 才发布 Connected，
/// 最后放行 reader。任一步失败都会丢弃 activation，reader 静默退出。
async fn connect_register_activate(
    mut ws: SharedWsClient,
    id: TransportId,
    registration_tx: mpsc::UnboundedSender<TransportRegistration<SharedWsClient>>,
) -> Result<(), PortError> {
    let activation = ws.connect().await?;
    let ws = Arc::new(ws);
    register_transport(&registration_tx, id, Arc::clone(&ws)).await?;
    activation.activate().await
}

/// pump 线程主体：建 current_thread runtime → block_on 装配 + 跑 host 泵核。
///
/// 装配（开 storage / 建 ports / connect WS / 发布 readiness）在 runtime 内 async 完成；
/// 任一步失败 → `ready_tx.send(Err(()))` 后线程退出（create 据此返回 null）。
pub fn run(args: PumpArgs) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all() // 漏 enable_all → spawn_blocking await / timer park 异常
        // blocking pool 上限封顶（PI-9 / G-11）：current_thread 默认 512，移动端弱网下 getaddrinfo
        // DNS 解析 + rusqlite spawn_blocking 高并发可让线程暴涨。32 ≈ READ_POOL + max_http_inflight
        // + 余量（防 DNS 风暴线程暴涨，非性能调优——封顶是安全闸，不是吞吐目标）。
        .max_blocking_threads(32)
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "FFI pump 建 current_thread runtime 失败");
            let _ = args.ready_tx.send(Err(()));
            return;
        }
    };

    runtime.block_on(async move {
        // SHADOW-NOTE: 内层 async 块到 `engine::run_engine_loop().await` 结束即 graceful drain
        // 完成；此后须 `shutdown_timeout` 收尾 runtime，绝不靠 Drop（见块末注释）。
        // ── storage（共享 host rusqlite）─────────────────────────────
        let storage = match HostStorage::open_sqlite_url(&args.db_url).await {
            Ok(s) => Arc::new(s.with_metric_sink(Arc::clone(&args.metrics))),
            Err(e) => {
                tracing::error!(error = %e, db = %args.db_url, "FFI pump 打开 storage 失败");
                let _ = args.ready_tx.send(Err(()));
                return;
            }
        };
        for &sql in helix_im::IM_SCHEMA {
            if let Err(e) = storage.execute_raw(sql).await {
                tracing::error!(
                    error = %e,
                    db = %args.db_url,
                    "FFI pump 初始化 IM schema 失败"
                );
                let _ = args.ready_tx.send(Err(()));
                return;
            }
        }
        for &sql in helix_im::IM_SCHEMA_MIGRATIONS {
            if let Err(e) = storage.execute_raw(sql).await {
                if !is_duplicate_column_error(&e) {
                    tracing::error!(
                        error = %e,
                        db = %args.db_url,
                        "FFI pump 初始化 IM schema migration 失败"
                    );
                    let _ = args.ready_tx.send(Err(()));
                    return;
                }
            }
        }

        // ── event sink（FFI 批处理 egress）先建：HTTP 业务 outcome 回报复用同一出口。
        let event_sink = Arc::new(
            FfiEventSink::new(args.vtable).with_metric_sink(Arc::clone(&args.metrics)),
        );
        let otel = HostOtelRuntime::from_env("helix-driver-ffi");

        // ── http / ws ports（Rust 持有，共享 header registry）──────────
        let http = match SharedHttpClient::with_registry(args.config.clone(), args.headers.clone())
        {
            Ok(c) => FfiHttpRequester::new(
                c.with_metric_sink(Arc::clone(&args.metrics)),
                Arc::clone(&event_sink),
                args.auth.clone(),
                helix_im::http_outcome_event,
            )
            .with_otel(otel.clone()),
            Err(e) => {
                tracing::error!(error = %e, "FFI pump 建 SharedHttpClient 失败");
                let _ = args.ready_tx.send(Err(()));
                return;
            }
        };
        let main_transport = TransportId::from_raw(MAIN_TRANSPORT);
        // 入站帧 → Tick::Inbound 直发泵（with_inbound_tick）；不经 C++。
        let ws = match SharedWsClient::with_registry(args.config.clone(), args.headers.clone()) {
            Ok(c) => c
                .with_metric_sink(Arc::clone(&args.metrics))
                .with_stamped_inbound_tick(main_transport, args.tick_tx.clone()),
            Err(e) => {
                tracing::error!(error = %e, "FFI pump 建 SharedWsClient 失败");
                let _ = args.ready_tx.send(Err(()));
                return;
            }
        };
        // 🔴 connect 异步化（fix/lifecycle-net-decouple）：**装配阶段绝不 await connect**——
        // 否则连不上的 host 会让 `helix_create_with_network` 同步阻塞满 `DEFAULT_TIMEOUT_SECS`
        // （实测 create 卡死直到 TCP connect 超时）。`ready_tx` 在 spawn 后台连接任务后立即回传 →
        // create 不等网络（约束3：create 非阻塞不回退）。
        //
        // ## 连接期句柄类型层不可见（消除锁跨 await，非靠时序不变量）
        //
        // 后台任务**独占 move 进一个裸 `SharedWsClient`**（无 Arc / 无 Mutex 包裹），对它跑
        // `connect().await`（连不上可达 ~30s）。此窗口内该句柄**根本不在 `transports` 表里**
        // （表起为空），dispatch 的 `Effect::Send` 走 `transports.get()=None` warn 兜底 → 不存在
        // 任何「会被 Send 争用、且被某任务持有跨 connect().await」的锁（**类型/可见性层**证明，
        // 而非旧实现靠「hello 之后才产 Send」的运行时时序不变量）。连成功后经 registration
        // 通道投进泵，loop 先 insert、再 ack；后台任务收到 ack 才发布 Connected/放行 reader。
        let (transport_tx, transport_rx) =
            mpsc::unbounded_channel::<TransportRegistration<SharedWsClient>>();
        let (transport_lifecycle_tx, transport_lifecycle_rx) = mpsc::unbounded_channel();
        let (reconnect_shutdown_tx, reconnect_shutdown_rx) = watch::channel(false);
        let transport_trace_tx = spawn_transport_trace_exporter(&otel);
        let transports: TransportTable<SharedWsClient> = TransportTable::new();
        // 后台连接任务：与泵核同跑此 current_thread runtime（spawn 进同 runtime 域）。
        // 初连失败只 warn；已连接后的 reader 断线则由下方 supervisor 按策略重连。
        let initial_transport_tx = transport_tx.clone();
        tokio::spawn(async move {
            match connect_register_activate(ws, main_transport, initial_transport_tx).await {
                Ok(()) => tracing::info!("FFI pump 主 WS 已注册并激活"),
                Err(e) => {
                    tracing::error!(error = %e, "FFI pump 主 WS 后台连接/注册/激活失败——仍进泵，上行走 warn 兜底")
                }
            }
        });
        let reconnect_config = args.config.clone();
        let reconnect_headers = args.headers.clone();
        let reconnect_tick_tx = args.tick_tx.clone();
        let reconnect_transport_tx = transport_tx.clone();
        let reconnect_metrics = Arc::clone(&args.metrics);
        let reconnect_handle = spawn_reconnect_supervisor(
            main_transport,
            ReconnectPolicy::default(),
            transport_lifecycle_rx,
            reconnect_shutdown_rx,
            ReconnectTraceSink::new(transport_trace_tx.clone()),
            move || {
                let config = reconnect_config.clone();
                let headers = reconnect_headers.clone();
                let tick_tx = reconnect_tick_tx.clone();
                let registration_tx = reconnect_transport_tx.clone();
                let metrics = Arc::clone(&reconnect_metrics);
                async move {
                    let ws = SharedWsClient::with_registry(config, headers)?
                        .with_metric_sink(metrics)
                        .with_stamped_inbound_tick(main_transport, tick_tx);
                    connect_register_activate(ws, main_transport, registration_tx).await
                }
            },
        );

        // ── clock（host 注入壁钟）────────────────────────────────────
        let clock = FfiClock::new(args.vtable.now_ms, args.vtable.ctx);

        // ── shell + IM 模块 ─────────────────────────────────────────
        let mut shell = ExecutionShell::new();
        shell.register(ImModule::new_with_local_store(
            ImConfig {
                ws_url: args.config.ws_url.clone(),
                api_base_url: args.config.api_base_url.clone(),
                // 第二网关 base（spec06 方案 A）：vote/score outbound 走它（HostNetworkConfig 已派生注入）。
                default_api_base_url: args.config.default_api_base_url.clone(),
                ..Default::default()
            },
            LocalStoreMode::Durable,
        ));

        // 装配成功 → 发布 readiness。storage 只进入共享 engine，不再暴露同步业务查询句柄。
        #[cfg(test)]
        let trace = args
            .trace_override
            .unwrap_or_else(|| ffi_trace_hooks(args.command_traces, otel));
        #[cfg(not(test))]
        let trace = ffi_trace_hooks(args.command_traces, otel);
        let deps = EngineDeps {
            storage: Arc::clone(&storage),
            http: Arc::new(http),
            uploader: Arc::new(SharedFileUploader::default()),
            event_sink,
            clock,
            trace,
            metrics: Arc::clone(&args.metrics),
            max_http_inflight: args.max_http_inflight,
            transport_lifecycle_tx: Some(transport_lifecycle_tx),
            transport_trace_tx,
        };
        let _ = args.ready_tx.send(Ok(()));

        // ── 跑 host 泛型泵核（graceful drain：shutdown_rx 或 tick_rx 关闭）──
        engine::run_engine_loop_stamped(
            shell,
            args.tick_rx,
            args.tick_tx,
            deps,
            args.shutdown_rx,
            transports,
            transport_rx,
        )
        .await;
        reconnect_shutdown_tx.send_replace(true);
        if let Err(error) = reconnect_handle.await {
            tracing::warn!(%error, "FFI reconnect supervisor 非正常退出");
        }
        if let Some(metric_runtime) = args.metric_runtime {
            let outcome = metric_runtime.shutdown(Duration::from_millis(1500));
            tracing::debug!(?outcome, "FFI 异步指标 worker 已收尾");
        }
    });

    // 🔴 lifecycle-net-decouple（生命周期绝不被网络 I/O 阻塞）：
    //
    // `run_engine_loop` 返回 = graceful drain 已完成（Persist 全落地 + 必达 Http 有上限 drain）。
    // 但**装配阶段 spawn 的后台 WS connect 任务**可能仍卡在 `connect_async().await` 的 DNS 解析
    // （`getaddrinfo` 经 tokio blocking pool）——连不上的 host 上单次解析可达系统 resolver 上限
    // （实测 ~30s）。若直接让 `runtime` 走 `Drop`，current_thread runtime 的 Drop 会**同步等待
    // 在途 blocking pool 线程结束** → `helix_destroy` 的 join pump 线程随之卡满 ~30s（cpp-smoke
    // 30s ctest TIMEOUT 正中此处）。
    //
    // 故 drain 完成后**显式 `shutdown_timeout(0)`** 收尾：detach 仍在途的 getaddrinfo blocking
    // 线程（进程随后整体释放 handle，孤儿 DNS 解析无副作用），pump 线程立即退出 → join 不阻塞。
    // 这与 engine 五不变量⑤「必达 Http drain 有上限、生命周期不被网络阻塞」同源——把同一红线
    // 补到 runtime 收尾这最后一环（drain 之后唯一可能残留的网络 I/O = 后台 connect 的 DNS）。
    runtime.shutdown_timeout(Duration::ZERO);
}

fn is_duplicate_column_error(e: &impl std::fmt::Display) -> bool {
    e.to_string().contains("duplicate column name")
}

#[cfg(test)]
#[path = "pump_lifecycle_tests.rs"]
mod lifecycle_tests;
