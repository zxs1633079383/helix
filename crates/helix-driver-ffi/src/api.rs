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

use bytes::Bytes;
use helix_core::tick::{AppCommand, Tick};
use helix_driver_host::{HostHeaderRegistry, HostNetworkConfig};
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

    let config = HostNetworkConfig::new(api_base_url, ws_url);
    let headers = HostHeaderRegistry::default();

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
    match handle.headers.set_header_sync(&name, &value) {
        Ok(()) => FFI_OK,
        Err(_) => FFI_INVALID,
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
    match handle.headers.remove_header_sync(&name) {
        Ok(()) => FFI_OK,
        Err(_) => FFI_INVALID,
    }
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
    let name_str = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };

    // 立即拷贝 payload（ACL-3：C 侧 buffer 调用返回后可能被复用）
    let payload_bytes: Bytes = if payload.is_null() || payload_len == 0 {
        Bytes::new()
    } else {
        // SAFETY: payload 非 null 已确认；调用方契约保证指向至少 payload_len 字节、
        // 在本调用期间有效的内存——立即 copy_from_slice 拷贝，不持有原始指针。
        Bytes::copy_from_slice(unsafe { std::slice::from_raw_parts(payload, payload_len) })
    };

    let cmd = AppCommand {
        name: Cow::Owned(name_str),
        payload: payload_bytes,
    };

    // SAFETY: handle 非 null 已检查；来自 helix_create_with_network。
    let handle_ref = unsafe { &*handle };
    let guard = handle_ref.tick_tx.lock();
    match guard.as_ref() {
        None => FFI_NULL, // helix_destroy 已调用，channel 已关闭
        Some(tx) => match tx.try_send(Tick::Command(cmd)) {
            Ok(()) => FFI_OK,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => FFI_BUSY,
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => FFI_NULL,
        },
    }
}

// ── helix_query / helix_query_free ───────────────────────────────────────────

/// 同步读取 engine 状态（**honest stub**：返回 `FFI_UNIMPLEMENTED`，基础设施已就位）。
///
/// ## 为什么仍是 stub（取舍说明）
///
/// 同步读的**运行时基础设施已接好**（`HelixHandle.query` = pump runtime `Handle` + 共享
/// `HostStorage`；C 同步线程可 `handle.spawn(async { storage.scan/get })` + `blocking_recv`
/// 等结果，阻塞 C 线程合规——它不是 tokio worker，HostStorage 内部已 `spawn_blocking`，不违
/// HX-C006 §5）。**缺的是 `query_name → ScanSpec/GetSpec` 的映射**——那是 IM 领域语义
/// （哪个表、哪些列、WHERE 谓词），属 helix-im 关注点。在 FFI 层硬编码该映射会把 IM schema
/// 泄漏进 driver（违反 runtime-agnostic-core.md §6「Storage trait 不许泄漏业务 schema」）。
/// 故保留 honest stub，待 IM 暴露通用 query 路由（或经 command + Emit 回读）后填充，不在此越界。
///
/// ## 配对约定（填充后生效）
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
    _query_name: *const u8,
    _query_name_len: usize,
    _query_payload: *const u8,
    _query_payload_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if handle.is_null() || out_ptr.is_null() || out_len.is_null() {
        return FFI_NULL;
    }
    // honest stub：infra（runtime handle + storage）已在 HelixHandle.query 就位；
    // 缺 query_name→Spec 映射（IM 领域语义），填充会泄漏 schema 进 driver → 暂不实现（见 doc）。
    FFI_UNIMPLEMENTED
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

// ── 测试 ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
// 测试体内调用本 crate 自己的 FFI 导出（unsafe extern "C"）：入参均为测试栈上构造的有效
// 指针/长度，安全性由测试自身保证，逐块 SAFETY 注释冗余无益 → 整 mod 豁免该 lint（仅测试）。
#[allow(clippy::undocumented_unsafe_blocks)]
mod tests {
    use super::*;
    use crate::vtable::CppHostVtable;

    extern "C" fn noop_events(_ctx: *mut std::ffi::c_void, _ptr: *const u8, _len: usize) {}
    extern "C" fn noop_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
        42
    }

    fn test_vtable() -> CppHostVtable {
        CppHostVtable {
            on_domain_events_batch: Some(noop_events),
            now_ms: Some(noop_now_ms),
            ctx: std::ptr::null_mut(),
        }
    }

    #[test]
    fn set_header_rejects_null_handle() {
        let name = b"authorization";
        let value = b"Bearer token";
        let status = unsafe {
            helix_set_header(
                std::ptr::null_mut(),
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            )
        };
        assert!(status < 0);
    }

    #[test]
    fn create_with_network_allows_set_and_remove_header() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(
                api.as_ptr(),
                api.len(),
                ws.as_ptr(),
                ws.len(),
                test_vtable(),
            )
        };
        assert!(!handle.is_null());

        let name = b"authorization";
        let value = b"Bearer token";
        let set_status = unsafe {
            helix_set_header(
                handle,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            )
        };
        assert_eq!(set_status, 0);

        let remove_status = unsafe { helix_remove_header(handle, name.as_ptr(), name.len()) };
        assert_eq!(remove_status, 0);

        unsafe { helix_destroy(handle) };
    }

    #[test]
    fn set_header_rejects_invalid_utf8() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(
                api.as_ptr(),
                api.len(),
                ws.as_ptr(),
                ws.len(),
                test_vtable(),
            )
        };
        assert!(!handle.is_null());

        let invalid = [0xff_u8, 0xfe];
        let value = b"Bearer token";
        let status = unsafe {
            helix_set_header(
                handle,
                invalid.as_ptr(),
                invalid.len(),
                value.as_ptr(),
                value.len(),
            )
        };
        assert_eq!(status, FFI_UTF8);

        unsafe { helix_destroy(handle) };
    }

    #[test]
    fn command_rejects_null_handle() {
        let name = b"test_cmd";
        let payload = b"{}";
        let status = unsafe {
            helix_command(
                std::ptr::null_mut(),
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        assert_eq!(status, FFI_NULL);
    }

    #[test]
    fn command_enqueues_successfully() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(
                api.as_ptr(),
                api.len(),
                ws.as_ptr(),
                ws.len(),
                test_vtable(),
            )
        };
        assert!(!handle.is_null());

        let name = b"send_message";
        let payload = b"{\"text\":\"hello\"}";
        let status = unsafe {
            helix_command(
                handle,
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        assert_eq!(status, FFI_OK);

        unsafe { helix_destroy(handle) };
    }

    #[test]
    fn query_returns_unimplemented() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(
                api.as_ptr(),
                api.len(),
                ws.as_ptr(),
                ws.len(),
                test_vtable(),
            )
        };
        assert!(!handle.is_null());

        let query_name = b"get_channels";
        let query_payload = b"{}";
        let mut out_ptr: *mut u8 = std::ptr::null_mut();
        let mut out_len: usize = 0;
        let status = unsafe {
            helix_query(
                handle,
                query_name.as_ptr(),
                query_name.len(),
                query_payload.as_ptr(),
                query_payload.len(),
                &mut out_ptr,
                &mut out_len,
            )
        };
        assert_eq!(status, FFI_UNIMPLEMENTED);

        unsafe { helix_destroy(handle) };
    }

    #[test]
    fn query_free_handles_null() {
        // null ptr → no-op, must not crash
        unsafe { helix_query_free(std::ptr::null_mut(), 0) };
    }

    // ── 端到端：命令 → step → Emit → 批回调 ────────────────────────────────────
    //
    // 用真 extern "C" vtable 录回调 bytes（ctx 指向 Mutex<Vec<Vec<u8>>>），证明
    // helix_command → pump step → Effect::Emit → FfiEventSink 攒批 → on_domain_events_batch
    // 端到端打通（批 wire 用 batch::decode_batch 还原，断言含 DomainEvent）。

    use std::sync::Mutex as StdMutex;

    /// 录批回调收到的「解码后各条 DomainEvent 原文」。ctx 指向它。
    type Recorder = StdMutex<Vec<Vec<u8>>>;

    extern "C" fn recording_events(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
        // SAFETY: ctx 是测试构造时传入的 &Recorder 的裸指针，在 handle 存活期间有效；
        // ptr/len 由 FfiEventSink::flush 传入，指向回调期间有效的 batch buffer。
        let rec = unsafe { &*(ctx as *const Recorder) };
        let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
        if let Some(events) = crate::batch::decode_batch(buf) {
            let mut g = rec.lock().unwrap();
            for e in events {
                g.push(e.to_vec());
            }
        }
    }

    extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
        1_700_000_000_000
    }

    /// 回归锚: M3-F4 —— 命令进 → 领域事件经批回调出（端到端打通证据）。
    ///
    /// 不设 HELIX_DB：每个 create 走 `ffi_db_url` 的进程+计数临时库 fallback（各 handle 独立库，
    /// 不与并发测试撞库、不污染共享 env）。
    #[test]
    fn command_emits_domain_event_through_batch_callback() {
        let recorder: Box<Recorder> = Box::new(StdMutex::new(Vec::new()));
        let ctx = &*recorder as *const Recorder as *mut std::ffi::c_void;
        let vtable = CppHostVtable {
            on_domain_events_batch: Some(recording_events),
            now_ms: Some(fixed_now_ms),
            ctx,
        };

        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
        };
        assert!(
            !handle.is_null(),
            "create 应成功（storage 开库 + pump 起线程）"
        );

        // im_send_message：同步 push Effect::Emit(im:post:sending)（不依赖网络成功）。
        let name = b"im_send_message";
        let payload =
            br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-1","text":"hi"}"#;
        let status = unsafe {
            helix_command(
                handle,
                name.as_ptr(),
                name.len(),
                payload.as_ptr(),
                payload.len(),
            )
        };
        assert_eq!(status, FFI_OK, "命令入队应成功");

        // 轮询等 pump step + flush 回调到达（pump 在独立线程异步处理）。
        let mut got = false;
        for _ in 0..200 {
            if !recorder.lock().unwrap().is_empty() {
                got = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(got, "应在超时内收到至少一条领域事件批回调");

        // 断言批里含 im:post:sending（命令 → Emit 端到端）。
        let events = recorder.lock().unwrap();
        let any_sending = events.iter().any(|e| {
            std::str::from_utf8(e)
                .map(|s| s.contains("im:post:sending"))
                .unwrap_or(false)
        });
        assert!(
            any_sending,
            "批回调应含 im:post:sending DomainEvent，实际收到 {} 条",
            events.len()
        );
        drop(events);

        // 销毁：graceful drain + join pump 线程（join 后 recorder/ctx 不再被回调，安全 drop）。
        unsafe { helix_destroy(handle) };
        // recorder 在此 drop——destroy 已 join pump，保证回调不再访问 ctx（无 use-after-free）。
    }

    /// pump 线程起停 + graceful drain 不挂（create → 立即 destroy，无命令）。
    #[test]
    fn pump_lifecycle_create_then_destroy_drains_cleanly() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(
                api.as_ptr(),
                api.len(),
                ws.as_ptr(),
                ws.len(),
                test_vtable(),
            )
        };
        assert!(!handle.is_null());
        // 立即 destroy：drop tick_tx + shutdown + join pump 线程，必须返回不挂死。
        unsafe { helix_destroy(handle) };
    }
}
