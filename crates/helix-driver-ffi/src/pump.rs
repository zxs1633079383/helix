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
        let mut ws = match SharedWsClient::with_registry(args.config.clone(), args.headers.clone())
        {
            Ok(c) => c.with_inbound_tick(main_transport, args.tick_tx.clone()),
            Err(e) => {
                tracing::error!(error = %e, "FFI pump 建 SharedWsClient 失败");
                let _ = args.ready_tx.send(None);
                return;
            }
        };
        // 连不上不退出——仍进泵，Effect::Send 走 host 「发送失败 warn」兜底（对齐 host-cli A1）。
        if let Err(e) = ws.connect().await {
            tracing::error!(error = %e, "FFI pump 主 WS 连接失败——仍进泵，上行走 warn 兜底");
        }
        let mut transports: TransportTable<SharedWsClient> = TransportTable::new();
        transports.insert(main_transport, Arc::new(tokio::sync::Mutex::new(ws)));

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
