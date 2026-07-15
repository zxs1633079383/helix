use helix_driver_host::{
    tick_ingress_channel, AsyncMetricSink, AuthTokenRegistry, BoundedMetricRuntime,
    CommandTraceQueue, HostHeaderRegistry, HostNetworkConfig, MetricRuntimeConfig, NoopMetricSink,
    TickIngressSender,
};
use std::sync::Arc;
use tokio::sync::oneshot;

use crate::pump::{self, PumpArgs};
use crate::vtable::CppHostVtable;

use super::shared::ffi_str;

// ── HelixHandle ──────────────────────────────────────────────────────────────

/// Helix 引擎实例句柄（opaque pointer，C++ 侧不透明）。
///
/// # 字段设计
///
/// - `headers`：运行时 header 快照（authorization 等），HTTP/WS 共享（pump 线程持同一 registry）。
/// - `tick_tx`：engine pump 输入端（`Mutex<Option<Sender>>`）。
///   `Option` 允许 destroy 时 `take()` 关闭 channel（pump recv None → graceful drain）；
///   `Mutex` 防 Sender 并发写（Sender 非 Sync）；**禁 Arc**——HelixHandle 本身即 Box opaque ptr。
/// - `shutdown_tx`：graceful drain 触发端（`Option`，destroy 时 take + send 一次）。
/// - `pump_thread`：pump 线程 JoinHandle（`Option`，destroy 时 take + join）。
/// - `query`：pump 装配成功后回传的 runtime handle + storage，供 `helix_query`（当前 stub）。
pub struct HelixHandle {
    pub(crate) headers: HostHeaderRegistry,
    /// HTTP 鉴权凭据 registry：`X-Auth-Kind` 只选路由，真凭据在 driver 侧热更新。
    pub(crate) auth: AuthTokenRegistry,
    /// `Tick::Command` 的 host-local trace sidecar FIFO（每个命令一槽，None/Some 都占位保序）。
    pub(crate) command_traces: CommandTraceQueue,
    /// 非阻塞指标入口；disabled 时为真 Noop，不创建 worker、不读时钟。
    pub(crate) metrics: Arc<dyn AsyncMetricSink>,
    /// tick channel 发送端。Sender 非 Sync，用 Mutex 包住允许 &self 送 tick。
    pub(crate) tick_tx: parking_lot::Mutex<Option<TickIngressSender>>,
    /// graceful drain 信号端；destroy 取出 send(()) 一次（与 tick_tx drop 双触发，任一即 drain）。
    pub(crate) shutdown_tx: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    /// pump 线程句柄；destroy 取出 join，确保 on_domain_events_batch 回调在 ctx 释放前全部完成。
    pub(crate) pump_thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
}

// ── helix_create_with_network ────────────────────────────────────────────────

/// 创建带 shared host network client 的 Helix 句柄（C++ 直连模型）。
///
/// 返回不透明指针；宿主必须持有并传给所有后续 API 调用，最终调用 `helix_destroy` 释放。
/// 失败时返回 null（字符串无效 / 网络客户端初始化失败）。
///
/// # Safety
///
/// - `api_base_url` / `ws_url`：非 null，各自指向至少对应 `_len` 字节、
///   在本函数调用期间有效的 UTF-8 内存。
/// - `vtable`：函数指针若非 None，在 `helix_destroy` 返回前必须保持有效（C++ 侧管生命周期）。
#[no_mangle]
pub unsafe extern "C" fn helix_create_with_network(
    api_base_url: *const u8,
    api_base_url_len: usize,
    ws_url: *const u8,
    ws_url_len: usize,
    vtable: CppHostVtable,
) -> *mut HelixHandle {
    // SAFETY: 调用方契约保证两个指针非 null 且各自指向对应 len 字节有效 UTF-8 内存。
    let Ok(api_base_url) = (unsafe { ffi_str(api_base_url, api_base_url_len) }) else {
        return std::ptr::null_mut();
    };
    // SAFETY: 同上。
    let Ok(ws_url) = (unsafe { ffi_str(ws_url, ws_url_len) }) else {
        return std::ptr::null_mut();
    };

    // 第二网关 base（spec06 方案 A）：vote/score 走 env.restHost（无 /api/cses）。C ABI 不新增参数
    // （不破 cbindgen 头/C++ 调用方）→ 由 api_base_url 去掉 `/api/cses` 尾段派生（与 host-cli 同策略）。
    let default_api_base_url = api_base_url.trim_end_matches("/api/cses").to_string();
    let mut config = HostNetworkConfig::new(api_base_url, ws_url);
    config.default_api_base_url = default_api_base_url;
    let headers = HostHeaderRegistry::default();
    let auth = AuthTokenRegistry::new();
    let command_traces = CommandTraceQueue::default();
    let metric_runtime = start_metric_runtime();
    let metrics = metric_runtime
        .as_ref()
        .map(BoundedMetricRuntime::sink)
        .unwrap_or_else(|| Arc::new(NoopMetricSink));

    // tick channel 容量 256：对齐 host-cli 入站背压点；满则 helix_command 返回 FFI_BUSY。
    let (tick_tx, tick_rx) = tick_ingress_channel(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    // 只等待 pump 完成 storage/schema/ports 装配，不暴露同步业务查询旁路。
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<(), ()>>(1);

    // DB 路径：env HELIX_DB（FFI host 设置）→ fallback 进程临时库（仅测试/无配置兜底）。
    let db_url = ffi_db_url();
    // Http 并发上限：env 注入，缺省/非正 → 8（对齐 host-cli）。
    let max_http_inflight = std::env::var("HELIX_HTTP_MAX_INFLIGHT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(8);

    let pump_args = PumpArgs {
        config,
        headers: headers.clone(),
        auth: auth.clone(),
        command_traces: command_traces.clone(),
        #[cfg(test)]
        trace_override: None,
        vtable,
        tick_tx: tick_tx.clone(),
        tick_rx,
        shutdown_rx,
        ready_tx,
        db_url,
        max_http_inflight,
        metrics: Arc::clone(&metrics),
        metric_runtime,
    };

    // pump 专用线程：current_thread runtime 跑 host run_engine_loop（core 单线程，GOAL §1.3）。
    let pump_thread = std::thread::Builder::new()
        .name("helix-ffi-pump".to_string())
        .spawn(move || pump::run(pump_args));
    let pump_thread = match pump_thread {
        Ok(t) => t,
        Err(e) => {
            tracing::error!(error = %e, "spawn FFI pump 线程失败");
            return std::ptr::null_mut();
        }
    };

    // 等 pump 装配结果。失败 → join + 返回 null。
    match ready_rx.recv() {
        Ok(Ok(())) => {}
        Ok(Err(())) | Err(_) => {
            // pump 装配失败已退出（或线程 panic 关闭 ready_tx）→ join 回收，返回 null。
            let _ = pump_thread.join();
            return std::ptr::null_mut();
        }
    }

    Box::into_raw(Box::new(HelixHandle {
        headers,
        auth,
        command_traces,
        metrics,
        tick_tx: parking_lot::Mutex::new(Some(tick_tx)),
        shutdown_tx: parking_lot::Mutex::new(Some(shutdown_tx)),
        pump_thread: parking_lot::Mutex::new(Some(pump_thread)),
    }))
}

fn start_metric_runtime() -> Option<BoundedMetricRuntime> {
    let enabled = std::env::var("HELIX_METRICS_ENABLED")
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"));
    if !enabled {
        return None;
    }
    let defaults = MetricRuntimeConfig::default();
    let config = MetricRuntimeConfig {
        enabled: true,
        service_name: "helix-driver-ffi".to_string(),
        endpoint: std::env::var("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")
            .unwrap_or_else(|_| defaults.endpoint.clone()),
        deployment_environment: std::env::var("DEPLOYMENT_ENVIRONMENT")
            .unwrap_or_else(|_| defaults.deployment_environment.clone()),
        platform: "ffi".to_string(),
        scenario: std::env::var("HELIX_METRIC_SCENARIO")
            .unwrap_or_else(|_| defaults.scenario.clone()),
        channel_shape: std::env::var("HELIX_METRIC_CHANNEL_SHAPE")
            .unwrap_or_else(|_| defaults.channel_shape.clone()),
        profile: std::env::var("HELIX_METRIC_PROFILE").unwrap_or_else(|_| defaults.profile.clone()),
        ..defaults
    };
    match BoundedMetricRuntime::start_otlp(config) {
        Ok(runtime) => Some(runtime),
        Err(error) => {
            tracing::error!(%error, "FFI 异步指标 worker 启动失败，降级为 Noop");
            None
        }
    }
}

/// FFI DB 路径解析：env `HELIX_DB` 显式优先；否则进程内临时库（测试/无配置兜底）。
///
/// 生产 FFI host 应经 env 注入多租户库路径（与 host-cli 派生逻辑对齐，本 stub 不重复派生）。
fn ffi_db_url() -> String {
    std::env::var("HELIX_DB").unwrap_or_else(|_| {
        let mut p = std::env::temp_dir();
        // 进程 + 单调计数避免并发 create 撞库（多 handle 各自独立库）。
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        p.push(format!("helix_ffi_{}_{}.db", std::process::id(), n));
        p.to_string_lossy().into_owned()
    })
}

// ── helix_destroy ────────────────────────────────────────────────────────────

/// 销毁 Helix 引擎实例，释放所有资源。调用后 `handle` 不可再使用。
///
/// ## 关闭顺序（graceful drain + join，独占所有权契约）
///
/// 1. 取出并 drop `tick_tx` → 关闭 tick channel（pump recv None 进 drain）。
/// 2. 取出并 send `shutdown_tx`（双触发冗余：drop tick_tx 与 shutdown 任一即触发 host graceful drain）。
/// 3. **join `pump_thread`**：阻塞到 pump 线程退出 → 保证最后一批 `on_domain_events_batch`
///    回调已返回，C++ 之后释放 vtable.ctx 安全（不会 use-after-free）。
///
/// # Safety
///
/// - `handle` 必须是 `helix_create_with_network` 返回的有效指针，或 null（null 时无操作）。
/// - 调用方必须保证此后不再向 `handle` 传递给任何 helix API（独占所有权契约，无 double-free）。
/// - **C++ 不得在本函数返回前释放 vtable.ctx**：join 完成前 pump 仍可能回调；本函数返回即安全。
#[no_mangle]
pub unsafe extern "C" fn helix_destroy(handle: *mut HelixHandle) {
    if handle.is_null() {
        return;
    }
    // SAFETY: handle 由 helix_create_with_network 的 Box::into_raw 产生，
    // 调用方契约保证单次 destroy（独占所有权，无 double-free）。
    let boxed = unsafe { Box::from_raw(handle) };

    // 1. 关闭 tick channel：pump recv() 返回 None → 进入 graceful drain。
    drop(boxed.tick_tx.lock().take());
    // 2. 冗余触发 shutdown（host biased select 任一信号即 drain；drop tick_tx 已足够，此为双保险）。
    if let Some(tx) = boxed.shutdown_tx.lock().take() {
        let _ = tx.send(());
    }
    // 3. join pump 线程：阻塞到泵退出（drain 完成）→ 最后一批 on_domain_events_batch 已返回，
    //    C++ 释放 ctx 安全。
    //    panic 隔离：pump 线程的 panic 由 std::thread 在线程边界捕获，join() 返回 Err 而非
    //    重新 panic——不存在 unwind 跨 extern "C"。tracing::error! 保证不 panic。
    //    ⚠️ 本 extern "C" 函数内禁止新增任何 panic 路径（panic!/expect/unwrap/poisoned-lock）：
    //       dev unwind 模式下 panic 越过 extern "C" = UB（HX-C006 §4）。
    if let Some(t) = boxed.pump_thread.lock().take() {
        if t.join().is_err() {
            tracing::error!("FFI pump 线程 panic（已在线程边界隔离，join 返回 Err）");
        }
    }

    drop(boxed);
}
