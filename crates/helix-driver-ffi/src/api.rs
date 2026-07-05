//! FFI 公开 API（#[no_mangle] C ABI 导出）
//!
//! ## C++ 直连模型
//!
//! - Rust 持有网络（SharedWsClient / SharedHttpClient）；C++ host 不喂帧/喂响应。
//! - 事件回传：pump 吐 Effect::Emit → FfiEventSink 攒批 → 每 step 单次
//!   `CppHostVtable::on_domain_events_batch`（HX-C007）。
//! - pump：`helix_create_with_network` 起专用线程（current_thread runtime）跑 host
//!   `run_engine_loop`；命令经 tick channel 进泵，事件经 vtable 回 C++。
//!
//! ## 内存安全约束（HX-C006）
//!
//! - 所有 *const u8 参数：**在函数返回前必须拷贝到 Rust 堆**（ACL-3）
//! - Box<T> raw ptr 必须用 Box::into_raw / Box::from_raw 配对，各一次
//! - panic=abort 保证不跨 FFI unwind（release profile 已设置）
//! - 每个 unsafe 块必须有 // SAFETY: 注释（clippy deny undocumented_unsafe_blocks）

use std::borrow::Cow;
use std::time::Duration;

use bytes::Bytes;
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, Tick};
use helix_core::AuthKind;
use helix_driver_host::{
    AuthTokenRegistry, CommandTraceQueue, HostHeaderRegistry, HostNetworkConfig, TraceCarrier,
};
use tokio::sync::{mpsc, oneshot};

use crate::pump::{self, PumpArgs, QueryDeps};
use crate::vtable::CppHostVtable;

// ── FFI 返回码 ──────────────────────────────────────────────────────────────
/// 操作成功
pub const FFI_OK: i32 = 0;
/// 传入 handle / 指针为 null
pub const FFI_NULL: i32 = -1;
/// 字节序列不是合法 UTF-8
pub const FFI_UTF8: i32 = -2;
/// 参数取值无效（header registry 错误等）
pub const FFI_INVALID: i32 = -3;
/// tick channel 满，调用方应稍后重试（M3-F4 pump 接入前此码不会实际出现）
pub const FFI_BUSY: i32 = -4;
/// 功能尚未实现（本轮留 stub，M3-F4 pump 接入后填充）
pub const FFI_UNIMPLEMENTED: i32 = -5;

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
    /// tick channel 发送端。Sender 非 Sync，用 Mutex 包住允许 &self 送 tick。
    pub(crate) tick_tx: parking_lot::Mutex<Option<mpsc::Sender<Tick>>>,
    /// graceful drain 信号端；destroy 取出 send(()) 一次（与 tick_tx drop 双触发，任一即 drain）。
    pub(crate) shutdown_tx: parking_lot::Mutex<Option<oneshot::Sender<()>>>,
    /// pump 线程句柄；destroy 取出 join，确保 on_domain_events_batch 回调在 ctx 释放前全部完成。
    pub(crate) pump_thread: parking_lot::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// 查询依赖（runtime handle + 共享 storage）。helix_query 当前为 honest stub，字段已就位。
    #[allow(dead_code)]
    pub(crate) query: QueryDeps,
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

    // tick channel 容量 256：对齐 host-cli 入站背压点；满则 helix_command 返回 FFI_BUSY。
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    // 装配回传通道（同步 rendezvous，容量 1）：pump 线程开 storage / 建 ports 后回传 QueryDeps。
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Option<QueryDeps>>(1);

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
        vtable,
        tick_tx: tick_tx.clone(),
        tick_rx,
        shutdown_rx,
        ready_tx,
        db_url,
        max_http_inflight,
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

    // 等 pump 装配结果（storage 打开 / ports 建好 / QueryDeps 回传）。装配失败 → join + 返回 null。
    let query = match ready_rx.recv() {
        Ok(Some(deps)) => deps,
        Ok(None) | Err(_) => {
            // pump 装配失败已退出（或线程 panic 关闭 ready_tx）→ join 回收，返回 null。
            let _ = pump_thread.join();
            return std::ptr::null_mut();
        }
    };

    Box::into_raw(Box::new(HelixHandle {
        headers,
        auth,
        command_traces,
        tick_tx: parking_lot::Mutex::new(Some(tick_tx)),
        shutdown_tx: parking_lot::Mutex::new(Some(shutdown_tx)),
        pump_thread: parking_lot::Mutex::new(Some(pump_thread)),
        query,
    }))
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

// ── helix_set_header / helix_remove_header ───────────────────────────────────

/// 更新 shared header registry。后续 HTTP 请求和 WS connect/reconnect 会读取最新值。
///
/// # Safety
///
/// - `handle`：helix_create_with_network 返回的有效指针，或 null（返回 FFI_NULL）。
/// - `name` / `value`：各自指向至少对应 `_len` 字节、在本函数调用期间有效的 UTF-8 内存。
#[no_mangle]
pub unsafe extern "C" fn helix_set_header(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    value: *const u8,
    value_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 name/value 各自指向对应 len 字节有效 UTF-8 内存。
    let name = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // SAFETY: 同上。
    let value = match unsafe { ffi_str(value, value_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };

    // SAFETY: handle 非 null 已检查；调用方契约保证来自 helix_create_with_network。
    let handle = unsafe { &*handle };
    match set_runtime_header(handle, &name, &value) {
        Ok(()) => FFI_OK,
        Err(()) => FFI_INVALID,
    }
}

/// 移除 shared header registry 中的 header。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name`：指向至少 `name_len` 字节有效 UTF-8 内存，在本调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn helix_remove_header(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 name 指向至少 name_len 字节有效 UTF-8 内存。
    let name = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };

    // SAFETY: handle 非 null 已检查；来自 helix_create_with_network。
    let handle = unsafe { &*handle };
    match remove_runtime_header(handle, &name) {
        Ok(()) => FFI_OK,
        Err(()) => FFI_INVALID,
    }
}

fn set_runtime_header(handle: &HelixHandle, name: &str, value: &str) -> Result<(), ()> {
    if name.eq_ignore_ascii_case("Authorization") {
        handle.auth.set_bot(name.to_string(), value.to_string());
        return Ok(());
    }

    handle
        .headers
        .set_header_sync(name, value)
        .map_err(|_| ())?;
    if name.eq_ignore_ascii_case("cookieId") {
        handle.auth.set_session(name.to_string(), value.to_string());
    }
    Ok(())
}

fn remove_runtime_header(handle: &HelixHandle, name: &str) -> Result<(), ()> {
    if name.eq_ignore_ascii_case("Authorization") {
        handle.auth.clear(AuthKind::Bot);
        return Ok(());
    }

    handle.headers.remove_header_sync(name).map_err(|_| ())?;
    if name.eq_ignore_ascii_case("cookieId") {
        handle.auth.clear(AuthKind::Session);
    }
    Ok(())
}

// ── helix_command ────────────────────────────────────────────────────────────

/// 向 engine 发送应用命令（UI → engine）。
///
/// 立即将 `name` / `payload` 拷贝到 Rust 堆（ACL-3），构造 `Tick::Command`，
/// 并 **非阻塞** 尝试送入 tick channel（pump 线程 rx 端消费 → step → Effect → 批回 C++）：
///   - channel 满 → 返回 `FFI_BUSY`（C++ 侧应稍后重试，禁静默丢弃）。
///   - destroy 后 channel 已关闭 → 返回 `FFI_NULL`。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name` / `payload`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
///   `name` 必须是合法 UTF-8；`payload` 为任意字节（不强要求 UTF-8）。
#[no_mangle]
pub unsafe extern "C" fn helix_command(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    payload: *const u8,
    payload_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 name 指向至少 name_len 字节有效 UTF-8 内存。
    let public_name = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let payload_vec = unsafe { ffi_payload_to_vec(payload, payload_len) };
    let cmd = match build_app_command(public_name, payload_vec) {
        Ok(cmd) => cmd,
        Err(code) => return code,
    };

    // SAFETY: handle 非 null 已检查；来自 helix_create_with_network。
    let handle_ref = unsafe { &*handle };
    send_command_tick(handle_ref, cmd, None)
}

/// 向 engine 发送带 trace sidecar 的应用命令（移动端 `__trace` / host sidecar 入口）。
///
/// 与 [`helix_command`] 共享完全相同的命令名别名和 payload 归一化路径；差别只在于多一个
/// driver-owned trace sidecar 槽位，供 host trace hook 在 `Tick::Command` 边界消费。
/// `Tick::Command` 结构本身保持不变，trace 不进入业务 payload。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name` / `payload` / `trace_json`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
/// - `trace_json` 必须是合法 UTF-8 的 JSON 文本；当前解析 W3C `traceparent` / `baggage`。
#[no_mangle]
pub unsafe extern "C" fn helix_command_with_trace(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    payload: *const u8,
    payload_len: usize,
    trace_json: *const u8,
    trace_json_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 name / trace_json 指向至少对应 len 字节有效 UTF-8 内存。
    let public_name = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // SAFETY: 同上。
    let trace_json = match unsafe { ffi_str(trace_json, trace_json_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let carrier = match TraceCarrier::from_json_str(&trace_json) {
        Ok(carrier) => carrier,
        Err(error) => {
            tracing::warn!(?error, "trace sidecar rejected");
            return FFI_INVALID;
        }
    };
    let payload_vec = unsafe { ffi_payload_to_vec(payload, payload_len) };
    let cmd = match build_app_command(public_name, payload_vec) {
        Ok(cmd) => cmd,
        Err(code) => return code,
    };

    // SAFETY: handle 非 null 已检查；来自 helix_create_with_network。
    let handle_ref = unsafe { &*handle };
    send_command_tick(handle_ref, cmd, Some(carrier))
}

fn build_app_command(public_name: String, payload_vec: Vec<u8>) -> Result<AppCommand, i32> {
    let normalized_payload = match helix_im::mobile_contract::normalize_command_payload(
        public_name.as_str(),
        &payload_vec,
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                command = public_name.as_str(),
                ?error,
                "mobile command payload rejected"
            );
            return Err(FFI_INVALID);
        }
    };
    let payload_bytes = Bytes::from(normalized_payload);

    let core_name = helix_im::mobile_contract::command_alias(public_name.as_str())
        .unwrap_or(public_name.as_str());

    // G-6 / PI-1：name 命中 outbound 闭集（88 命令）→ `Cow::Borrowed` 静态名免堆分配（热路径写命令
    // 净分配仅 payload 1 次）；miss（im_send_message/im_reconnect/query 名）→ `Cow::Owned` fallback
    // （低频，仍正确）。闭集真源在 helix-im registry（driver 不复制清单，避免漂移）。
    let name: Cow<'static, str> = match helix_im::outbound::canonical_command_name(core_name) {
        Some(static_name) => Cow::Borrowed(static_name),
        None => Cow::Owned(core_name.to_string()),
    };
    Ok(AppCommand {
        name,
        payload: payload_bytes,
    })
}

fn send_command_tick(handle: &HelixHandle, cmd: AppCommand, trace: Option<TraceCarrier>) -> i32 {
    let guard = handle.tick_tx.lock();
    let Some(tx) = guard.as_ref() else {
        return FFI_NULL;
    };

    // `tick_tx` 的锁把「trace slot 入队 + command try_send」序列化成同一临界区，保证 mixed
    // `helix_command` / `helix_command_with_trace` 并发时 FIFO 与 channel 入队顺序一致。
    handle.command_traces.push_slot(trace);
    match tx.try_send(Tick::Command(cmd)) {
        Ok(()) => FFI_OK,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            handle.command_traces.rollback_last();
            FFI_BUSY
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            handle.command_traces.rollback_last();
            FFI_NULL
        }
    }
}

// ── helix_query / helix_query_free ───────────────────────────────────────────

/// 同步读取 engine 状态。
///
/// ## 查询边界
///
/// 元信息 query 由 FFI 直接返回：
/// - `helix_capabilities`：返回移动端公开契约能力集 JSON
/// - `helix_version`：返回 `{"abi":1}`
///
/// IM 本地同步 query 由 `helix-im` 负责 `query_name → ScanSpec + response shape`：
/// - `im_query_dialog_list`
/// - `im_query_messages_by_channel`
///
/// `driver-ffi` 只执行 helix-im 产出的 Storage scan 计划并封装返回 buffer，不在 driver 层硬编码
/// IM 表名/列名/WHERE 语义，避免业务 schema 泄漏进平台驱动。
///
/// ## 配对约定
///
/// 成功（返回 0）时，`*out_ptr` 指向 Rust 分配的 boxed slice；
/// C++ 侧**必须**调用 `helix_query_free(*out_ptr, *out_len)` 释放，禁止 free() / delete[]。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `query_name` / `query_payload`：各自指向对应 `_len` 字节有效内存，在本调用期间有效。
/// - `out_ptr` / `out_len`：非 null，指向调用方分配的存储位置。
#[no_mangle]
pub unsafe extern "C" fn helix_query(
    handle: *mut HelixHandle,
    query_name: *const u8,
    query_name_len: usize,
    query_payload: *const u8,
    query_payload_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if handle.is_null() || out_ptr.is_null() || out_len.is_null() {
        return FFI_NULL;
    }
    // G-10：返回 FFI_UNIMPLEMENTED 前先清零 out 参数——C++ 侧若误读 *out_ptr/*out_len（无视返回码）
    // 得到 (null, 0) 而非未初始化栈垃圾，杜绝对随机指针 helix_query_free（UB）。out_ptr/out_len 非 null
    // 已确认。
    // SAFETY: out_ptr/out_len 非 null 已检查；调用方契约保证各指向其分配的可写存储位置。
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    // SAFETY: query_name 指针/长度由调用方保证在本调用期间有效，且 name 必须是 UTF-8。
    let query_name = match unsafe { ffi_str(query_name, query_name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let query_payload = if query_payload.is_null() || query_payload_len == 0 {
        &[][..]
    } else {
        // SAFETY: query_payload 指针/长度由调用方保证在本调用期间有效。
        unsafe { std::slice::from_raw_parts(query_payload, query_payload_len) }
    };

    match query_name.as_str() {
        // SAFETY: out_ptr/out_len 非 null 已检查；write_query_bytes 只写回新分配的 boxed slice 指针与长度。
        "helix_capabilities" => unsafe {
            write_query_bytes(
                helix_im::mobile_contract::capabilities_json(),
                out_ptr,
                out_len,
            )
        },
        // SAFETY: out_ptr/out_len 非 null 已检查；write_query_bytes 只写回新分配的 boxed slice 指针与长度。
        "helix_version" => unsafe { write_query_bytes(br#"{"abi":1}"#.to_vec(), out_ptr, out_len) },
        _ => {
            let plan = match helix_im::mobile_query::plan(query_name.as_str(), query_payload) {
                Ok(plan) => plan,
                Err(_) => return FFI_UNIMPLEMENTED,
            };
            // SAFETY: handle 非 null 已检查；调用方契约保证来自 helix_create_with_network。
            let handle_ref = unsafe { &*handle };
            let runtime = handle_ref.query.runtime.clone();
            let storage = handle_ref.query.storage.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            runtime.spawn(async move {
                let result = match plan {
                    helix_im::mobile_query::MobileQueryPlan::Scan { spec, shape } => storage
                        .scan(spec)
                        .await
                        .map(|rows| helix_im::mobile_query::shape_json(shape, rows)),
                };
                let _ = tx.send(result);
            });

            match rx.recv_timeout(Duration::from_secs(3)) {
                // SAFETY: out_ptr/out_len 非 null 已检查；bytes 所有权在此转交给 write_query_bytes 封装成返回缓冲区。
                Ok(Ok(bytes)) => unsafe { write_query_bytes(bytes, out_ptr, out_len) },
                Ok(Err(error)) => {
                    tracing::warn!(
                        query = query_name.as_str(),
                        error = ?error,
                        "mobile sync query storage failed"
                    );
                    FFI_INVALID
                }
                Err(_) => FFI_BUSY,
            }
        }
    }
}

/// 释放 `helix_query` 返回的 boxed slice。
///
/// 仅用于释放 `helix_query` 成功时通过 out_ptr/out_len 返回的内存，
/// 禁止用于任何其他 Rust 分配的内存（UB）。
///
/// # Safety
///
/// - `ptr`：必须是 `helix_query` 通过 `out_ptr` 写入的值，或 null（null 时无操作）。
/// - `len`：必须与 `helix_query` 写入 `out_len` 的值完全一致。
/// - 调用后 `ptr` 失效，不可再访问（禁 double-free）。
#[no_mangle]
pub unsafe extern "C" fn helix_query_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(ptr, len);
    // SAFETY: ptr 由 Box::<[u8]>::into_raw(boxed_slice) 产生（helix_query 约定）；
    // len 由调用方通过 out_len 取得，与 boxed_slice 长度完全一致。
    // slice_from_raw_parts_mut + Box::from_raw 重建 Box<[u8]> 后 drop 正确释放。
    drop(unsafe { Box::from_raw(slice) });
}

unsafe fn write_query_bytes(bytes: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    let mut boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    let ptr = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    // SAFETY: out_ptr/out_len 已由调用者提供且在 helix_query 中检查非 null；这里仅写回 boxed slice 元信息。
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
    FFI_OK
}

// ── 内部工具 ─────────────────────────────────────────────────────────────────

/// 将 C 侧裸指针 + 长度转换为 Rust `String`，立即拷贝（ACL-3）。
///
/// # Safety
///
/// 调用方保证 `ptr` 非 null 时指向至少 `len` 字节有效内存，且仅在本次调用期间有效。
unsafe fn ffi_str(ptr: *const u8, len: usize) -> Result<String, i32> {
    if ptr.is_null() {
        return Err(FFI_NULL);
    }
    // SAFETY: ptr 非 null 已确认；调用方契约保证指向至少 len 字节有效内存。
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| FFI_UTF8)
}

unsafe fn ffi_payload_to_vec(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: ptr 非 null 且调用方契约保证指向至少 len 字节有效内存；这里立即拷贝，不持有原指针。
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
