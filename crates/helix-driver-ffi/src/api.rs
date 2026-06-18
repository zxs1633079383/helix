//! FFI 公开 API（#[no_mangle] C ABI 导出）
//!
//! ## C++ 直连模型
//!
//! - Rust 持有网络（SharedWsClient / SharedHttpClient）；C++ host 不喂帧/喂响应。
//! - 事件回传：pump（M3-F4）吐 Effect::Emit → 序列化整批 → 单次 CppHostVtable::on_domain_events_batch。
//! - 本轮范围：仅边界层（HelixHandle + 无 pump 的 tick channel + 导出函数骨架）。
//!   pump / step 调用 / Effect 分发均留 TODO(M3-F4)。
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
use helix_driver_host::{HostHeaderRegistry, HostNetworkConfig, SharedHttpClient, SharedWsClient};
use tokio::sync::mpsc;

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
/// - `headers`：运行时 header 快照（authorization 等），HTTP/WS 共享。
/// - `http` / `ws`：Rust 持有的共享网络客户端，C++ 不感知。
/// - `cpp_host`：C++ 回调表，pump（M3-F4）回传领域事件时使用。
/// - `tick_tx`：engine pump 输入端（Mutex<Option<Sender>>）。
///   `Mutex` 包 `Option`：Option 允许 destroy 时 take() 关闭 channel；
///   `Mutex` 防 tick_tx 被并发写（Sender 不是 Sync）；
///   **禁用 `Arc`**——HelixHandle 本身就是 Box<T> opaque ptr，无需再一层 Arc。
/// - `_tick_rx`：pump（M3-F4）消费端；本轮无 pump，字段持住 rx 确保 channel 不提前关闭。
///   TODO(M3-F4): pump 线程建好后，从此字段取出 rx 传入 pump 主循环，此字段可删。
pub struct HelixHandle {
    pub(crate) headers: HostHeaderRegistry,
    /// Rust 持有 HTTP 客户端生命周期；M3-F4 pump 接入后通过 Effect::Http 发出请求。
    #[allow(dead_code)]
    pub(crate) http: SharedHttpClient,
    /// Rust 持有 WS 客户端生命周期；M3-F4 pump 接入后 FfiTransport 消费。
    #[allow(dead_code)]
    pub(crate) ws: SharedWsClient,
    /// C++ host 回调表；M3-F4 pump 接入后用于 on_domain_events_batch 回传。
    #[allow(dead_code)]
    pub(crate) cpp_host: CppHostVtable,
    /// tick channel 发送端。Sender 非 Sync，用 Mutex 包住允许 &self 送 tick。
    pub(crate) tick_tx: parking_lot::Mutex<Option<mpsc::Sender<Tick>>>,
    /// tick channel 接收端，由 pump（M3-F4）取走消费。本轮仅持住防止 channel 提前关闭。
    pub(crate) _tick_rx: mpsc::Receiver<Tick>,
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
    let Ok(http) = SharedHttpClient::with_registry(config.clone(), headers.clone()) else {
        return std::ptr::null_mut();
    };
    let Ok(ws) = SharedWsClient::with_registry(config, headers.clone()) else {
        return std::ptr::null_mut();
    };

    // tick channel 容量 64：本轮 pump 未建，rx 端无消费者；
    // 容量留够避免 helix_command 立刻返回 FFI_BUSY。
    // TODO(M3-F4): pump 接入后按实测 QPS 调整容量。
    let (tick_tx, _tick_rx) = mpsc::channel::<Tick>(64);

    Box::into_raw(Box::new(HelixHandle {
        headers,
        http,
        ws,
        cpp_host: vtable,
        tick_tx: parking_lot::Mutex::new(Some(tick_tx)),
        _tick_rx,
    }))
}

// ── helix_destroy ────────────────────────────────────────────────────────────

/// 销毁 Helix 引擎实例，释放所有资源。调用后 `handle` 不可再使用。
///
/// # Safety
///
/// - `handle` 必须是 `helix_create_with_network` 返回的有效指针，或 null（null 时无操作）。
/// - 调用方必须保证此后不再向 `handle` 传递给任何 helix API（独占所有权契约）。
/// - **C++ 不得在本函数返回前释放 vtable.ctx**：pump（M3-F4）在 join 完成前仍可能回调。
///   TODO(M3-F4): pump 线程建好后，在此处 join(pump_thread) 后再返回，
///   保证 on_domain_events_batch 不会在 ctx 释放后被调用。
#[no_mangle]
pub unsafe extern "C" fn helix_destroy(handle: *mut HelixHandle) {
    if handle.is_null() {
        return;
    }
    // SAFETY: handle 由 helix_create_with_network 的 Box::into_raw 产生，
    // 调用方契约保证单次 destroy（独占所有权，无 double-free）。
    let boxed = unsafe { Box::from_raw(handle) };

    // 取出并 drop sender → 关闭 tick channel。
    // pump（M3-F4）的 recv() 会随即返回 None → pump 主循环退出。
    drop(boxed.tick_tx.lock().take());

    // TODO(M3-F4): join pump_thread handle（从 HelixHandle 取出），
    // 确保 on_domain_events_batch 回调在 vtable.ctx 释放前全部完成。

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
/// 并 **非阻塞** 尝试送入 tick channel：
///   - channel 满 → 返回 `FFI_BUSY`（C++ 侧应稍后重试，禁静默丢弃）。
///   - pump 未建时命令仅入队不被消费，等 M3-F4 pump 接入后自动处理。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name` / `payload`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
///   `name` 必须是合法 UTF-8；`payload` 为任意字节（不强要求 UTF-8）。
///
/// # TODO(M3-F4)
///
/// pump 接入前命令仅入队不被消费；pump 接入后 tick channel rx 端被 pump 主循环驱动。
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

/// 同步读取 engine 状态（stub，M3-F4 接入后填充）。
///
/// 本轮返回 `FFI_UNIMPLEMENTED`，不写 out_ptr / out_len。
///
/// # 配对约定
///
/// 成功（返回 0）时，`*out_ptr` 指向 Rust 分配的 boxed slice；
/// C++ 侧**必须**调用 `helix_query_free(*out_ptr, *out_len)` 释放，禁止 free() / delete[]。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `query_name` / `query_payload`：各自指向对应 `_len` 字节有效内存，在本调用期间有效。
/// - `out_ptr` / `out_len`：非 null，指向调用方分配的存储位置。
///
/// # TODO(M3-F4)
///
/// 实现：经 pump runtime Handle 的 `spawn_blocking` 跑 Storage 同步读
/// （HX-C006 §5 禁止在 tokio worker 直接 block_on）→ 序列化结果
/// → `into_boxed_slice()` → `Box::into_raw(slice)` 写 out_ptr / out_len（配对 helix_query_free）。
/// **禁止** 在本函数内 block_on（死锁：若 tokio 运行时在同一线程，block_on 会死锁）。
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
    // TODO(M3-F4): 经 pump runtime Handle spawn_blocking 跑 Storage 同步读；
    // 序列化结果 → into_boxed_slice → Box::into_raw 写 out_ptr/out_len（配对 helix_query_free）。
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

        let remove_status =
            unsafe { helix_remove_header(handle, name.as_ptr(), name.len()) };
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
}
