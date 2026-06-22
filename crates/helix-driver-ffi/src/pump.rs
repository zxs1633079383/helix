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

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::engine::{self, EngineDeps, TransportTable};
use helix_driver_host::{
    HostHeaderRegistry, HostNetworkConfig, HostStorage, SharedHttpClient, SharedWsClient,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use tokio::sync::{mpsc, oneshot};

use crate::clock::FfiClock;
use crate::event_sink::FfiEventSink;
use crate::http::FfiHttpRequester;
use crate::vtable::CppHostVtable;

/// 主 WS TransportId（host 集中分配；FFI 单连接固定 0，对齐 host-cli 约定）。
const MAIN_TRANSPORT: u32 = 0;

/// pump 线程回传给 `HelixHandle` 的查询依赖（helix_query 用）。
///
/// runtime `Handle` 让 C 同步线程 `spawn` 异步读到 pump runtime（不在 C 线程 block_on）；
/// storage 是同一个共享实例（pump 与 query 读同库）。
pub struct QueryDeps {
    pub runtime: tokio::runtime::Handle,
    pub storage: Arc<HostStorage>,
}

/// pump 线程入口参数（create 在主线程构造，move 进线程）。
pub struct PumpArgs {
    pub config: HostNetworkConfig,
    pub headers: HostHeaderRegistry,
    pub vtable: CppHostVtable,
    pub tick_tx: mpsc::Sender<Tick>,
    pub tick_rx: mpsc::Receiver<Tick>,
    pub shutdown_rx: oneshot::Receiver<()>,
    /// 装配成功后回传 QueryDeps（storage 打开/runtime handle）；失败回传 None。
    pub ready_tx: std::sync::mpsc::SyncSender<Option<QueryDeps>>,
    pub db_url: String,
    pub max_http_inflight: usize,
}

/// pump 线程主体：建 current_thread runtime → block_on 装配 + 跑 host 泵核。
///
/// 装配（开 storage / 建 ports / connect WS / 回传 QueryDeps）在 runtime 内 async 完成；
/// 任一步失败 → `ready_tx.send(None)` 后线程退出（create 据此返回 null）。
pub fn run(args: PumpArgs) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all() // 漏 enable_all → spawn_blocking await / timer park 异常
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "FFI pump 建 current_thread runtime 失败");
            let _ = args.ready_tx.send(None);
            return;
        }
    };

    runtime.block_on(async move {
        // ── storage（共享 host rusqlite）─────────────────────────────
        let storage = match HostStorage::open_sqlite_url(&args.db_url).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                tracing::error!(error = %e, db = %args.db_url, "FFI pump 打开 storage 失败");
                let _ = args.ready_tx.send(None);
                return;
            }
        };

        // ── http / ws ports（Rust 持有，共享 header registry）──────────
        let http = match SharedHttpClient::with_registry(args.config.clone(), args.headers.clone())
        {
            Ok(c) => FfiHttpRequester::new(c),
            Err(e) => {
                tracing::error!(error = %e, "FFI pump 建 SharedHttpClient 失败");
                let _ = args.ready_tx.send(None);
                return;
            }
        };
        let main_transport = TransportId::from_raw(MAIN_TRANSPORT);
        // 入站帧 → Tick::Inbound 直发泵（with_inbound_tick）；不经 C++。
        let ws = match SharedWsClient::with_registry(args.config.clone(), args.headers.clone()) {
            Ok(c) => c.with_inbound_tick(main_transport, args.tick_tx.clone()),
            Err(e) => {
                tracing::error!(error = %e, "FFI pump 建 SharedWsClient 失败");
                let _ = args.ready_tx.send(None);
                return;
            }
        };
        // 🔴 connect 异步化（fix/lifecycle-net-decouple）：**装配阶段绝不 await connect**——
        // 否则连不上的 host 会让 `helix_create_with_network` 同步阻塞满 `DEFAULT_TIMEOUT_SECS`
        // （实测 create 卡死直到 TCP connect 超时）。ws 先进 transports table，再 spawn 后台连接
        // 任务，`ready_tx` 立即回传 → create 不等网络（兑现 :109 旧注释「连不上仍进泵」意图：
        // 真正做到「立即进泵、连接后台进行」，而非旧实现的 `await` 同步等满）。
        //
        // 未连上时的出站语义（已正确，无需额外处理）：`Effect::Send` 在 host `dispatch` 里对
        // `WsState::Disconnected` 走 `send()→Err` 的「发送失败 warn」兜底（dispatch.rs 连接异常分支），
        // 不 panic、不阻塞泵。连上后读任务把入站帧作 `Tick::Inbound` 直发泵（with_inbound_tick）。
        let ws_handle = Arc::new(tokio::sync::Mutex::new(ws));
        let mut transports: TransportTable<SharedWsClient> = TransportTable::new();
        transports.insert(main_transport, Arc::clone(&ws_handle));
        // 后台连接任务：与泵核同跑此 current_thread runtime（spawn 进同 runtime 域）。
        // 连不上仅 warn——上行走 dispatch 的 warn 兜底，连接由读任务/后续重连推进（YAGNI：
        // 当前不内建退避重连，对齐 host-cli「连不上仍进泵」最简语义；重连留 IM 层 im_reconnect 命令驱动）。
        //
        // ⚠️ 锁跨 connect().await 的死锁/停顿分析（五不变量② 主循环不等 I/O 的守护）：
        // 此任务持 `ws_handle`（= transports[main] 同一 Arc<Mutex>）跨 `connect().await`（连不上
        // 可达 ~30s）。理论上若此窗口内泵分流 `Effect::Send`，dispatch 的 `t.lock().await` 会等满
        // → 泵停顿。**实际不发生**：IM 的 `Effect::Send`（hello-ack/ping/业务 WS 帧）只在 transport
        // 报 Connected + 服务端下发 hello **之后**才产出（module.rs handle_hello 链；ping 在 hello 后
        // 才 ScheduleTimer）。而 hello 经读任务 `Tick::Inbound` 进泵——读任务只在 connect 成功后才
        // 启动。故 connect 在途（未成功）时**无任何 Send 入泵**，锁无人争用；connect 成功瞬间锁即
        // 释放，其后的 Send 畅通。on_start 产 Scan/Http（不走此锁），亦不受影响。
        //
        // 🟡 drift-review(wf_efcf1ca4-759 #4) 收敛点：上述安全性靠**运行时时序不变量**（hello-gated
        // Send），非类型强制——属架构脆弱点。类型安全的结构收敛 = 「connect 成功后再把 main_transport
        // 插入 transports，连接期间表里无它，dispatch 走 None 兜底 warn」。但 `TransportTable<Tr>` =
        // `HashMap`（engine.rs:79）**按值 move 进 `run_engine_loop`**（不可在 loop 启动后插入）——
        // 该收敛需把 transports 改成跨 loop+连接任务共享形态（engine-core API 变更，native+ffi+pump 核
        // 同改 + 每 Send 加锁的性价比评估）。**留 ROADMAP-P 排期**（docs/ROADMAP.md「FFI pump 连接-插表
        // 时序类型化」），不在本 HX-C005 回正批内做风险重构。现状靠本注释 + 单线程 runtime + hello-gate
        // 三重保障，无实测停顿（lifecycle_net_decouple 测覆盖 create/destroy 不阻塞）。
        tokio::spawn(async move {
            if let Err(e) = ws_handle.lock().await.connect().await {
                tracing::error!(error = %e, "FFI pump 主 WS 后台连接失败——仍进泵，上行走 warn 兜底");
            }
        });

        // ── event sink（FFI 批处理 egress）+ clock（host 注入壁钟）────────
        let event_sink = FfiEventSink::new(args.vtable);
        let clock = FfiClock::new(args.vtable.now_ms, args.vtable.ctx);

        // ── shell + IM 模块 ─────────────────────────────────────────
        let mut shell = ExecutionShell::new();
        shell.register(ImModule::new(ImConfig {
            ws_url: args.config.ws_url.clone(),
            api_base_url: args.config.api_base_url.clone(),
            ..Default::default()
        }));

        // 装配成功 → 回传 QueryDeps（runtime handle + storage 共享给 helix_query）。
        let deps = EngineDeps {
            storage: Arc::clone(&storage),
            http: Arc::new(http),
            event_sink: Arc::new(event_sink),
            clock,
            max_http_inflight: args.max_http_inflight,
        };
        let _ = args.ready_tx.send(Some(QueryDeps {
            runtime: tokio::runtime::Handle::current(),
            storage,
        }));

        // ── 跑 host 泛型泵核（graceful drain：shutdown_rx 或 tick_rx 关闭）──
        engine::run_engine_loop(
            shell,
            args.tick_rx,
            args.tick_tx,
            deps,
            args.shutdown_rx,
            transports,
        )
        .await;
    });
}
