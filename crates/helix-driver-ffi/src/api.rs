//! FFI 公开 API（#[no_mangle] C ABI 导出）
//!
//! ## 内存安全约束（必须遵守）
//!
//! - 所有 *const u8 参数：**在函数返回前必须拷贝到 Rust 堆**
//! - 所有 Box<T> → raw ptr 必须用 Box::into_raw / Box::from_raw 配对
//! - panic=abort 保证不跨 FFI unwind（无需 catch_unwind，但不得 panic）

use crate::callbacks::HelixCallbacks;
use helix_driver_host::{HostHeaderRegistry, HostNetworkConfig, SharedHttpClient, SharedWsClient};

const FFI_OK: i32 = 0;
const FFI_NULL: i32 = -1;
const FFI_UTF8: i32 = -2;
const FFI_INVALID: i32 = -3;

pub struct HelixHandle {
    headers: HostHeaderRegistry,
    _http: SharedHttpClient,
    _ws: SharedWsClient,
    _callbacks: HelixCallbacks,
}

/// 创建 Helix 引擎实例，返回不透明指针。
/// 宿主负责持有并传递给所有后续 API 调用。
#[no_mangle]
pub extern "C" fn helix_create(_callbacks: HelixCallbacks) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

/// 创建带 shared host network client 的 Helix 句柄。
///
/// # Safety
///
/// 调用方必须保证非空指针均指向至少对应 len 字节、且在本函数调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn helix_create_with_network(
    api_base_url: *const u8,
    api_base_url_len: usize,
    ws_url: *const u8,
    ws_url_len: usize,
    callbacks: HelixCallbacks,
) -> *mut HelixHandle {
    let Some(api_base_url) = ffi_str(api_base_url, api_base_url_len).ok() else {
        return std::ptr::null_mut();
    };
    let Some(ws_url) = ffi_str(ws_url, ws_url_len).ok() else {
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

    Box::into_raw(Box::new(HelixHandle {
        headers,
        _http: http,
        _ws: ws,
        _callbacks: callbacks,
    }))
}

/// 销毁 Helix 引擎实例（释放内存）。
/// 调用后 handle 不可再使用。
#[no_mangle]
pub extern "C" fn helix_destroy(handle: *mut std::ffi::c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: `helix_create_with_network` transfers this Box to C. `helix_destroy`
    // is the only Rust API that reclaims it.
    let _ = unsafe { Box::from_raw(handle.cast::<HelixHandle>()) };
}

/// 更新 shared header registry。后续 HTTP 请求和 WS connect/reconnect 会读取最新值。
///
/// # Safety
///
/// 调用方必须保证非空指针均指向至少对应 len 字节、且在本函数调用期间有效。
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
    let name = match ffi_str(name, name_len) {
        Ok(name) => name,
        Err(code) => return code,
    };
    let value = match ffi_str(value, value_len) {
        Ok(value) => value,
        Err(code) => return code,
    };

    // SAFETY: handle non-null checked; caller contract says it came from Helix.
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
/// 调用方必须保证非空指针均指向至少对应 len 字节、且在本函数调用期间有效。
#[no_mangle]
pub unsafe extern "C" fn helix_remove_header(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    let name = match ffi_str(name, name_len) {
        Ok(name) => name,
        Err(code) => return code,
    };

    // SAFETY: handle non-null checked; caller contract says it came from Helix.
    let handle = unsafe { &*handle };
    match handle.headers.remove_header_sync(&name) {
        Ok(()) => FFI_OK,
        Err(_) => FFI_INVALID,
    }
}

/// 宿主收到 WS 帧时调用：把帧注入 engine。
/// 注意：data 指向的 buffer 必须在本函数内拷贝，不得持有。
///
/// # Safety
///
/// 调用方必须保证：`handle` 为 `helix_create` 返回的有效句柄或 null；
/// `data` 非 null 时指向至少 `len` 字节、在本调用期间有效的内存。
#[no_mangle]
pub unsafe extern "C" fn helix_on_ws_frame(
    handle: *mut std::ffi::c_void,
    data: *const u8,
    len: usize,
) {
    if handle.is_null() || data.is_null() {
        return;
    }
    // 立即拷贝（ACL-3：C 侧 buffer 回调返回后可能被复用）
    // SAFETY: data 非空已检查；调用方契约保证 data 指向至少 len 字节的有效内存，
    // 且仅在本回调期间有效——因此 from_raw_parts 后立即 to_vec 拷贝、不持有切片。
    let _owned: Vec<u8> = unsafe { std::slice::from_raw_parts(data, len).to_vec() };
    // TODO: 包成 Tick::Inbound 并通过 channel 发给 engine_loop
}

/// 宿主 HTTP 响应回调
///
/// # Safety
///
/// 调用方必须保证：`handle` 有效或 null；`body` 非 null 时指向至少
/// `body_len` 字节、在本调用期间有效的内存。
#[no_mangle]
pub unsafe extern "C" fn helix_on_http_response(
    handle: *mut std::ffi::c_void,
    _req_id: u64,
    _status: u16,
    body: *const u8,
    body_len: usize,
) {
    if handle.is_null() {
        return;
    }
    let _body_owned: Vec<u8> = if body.is_null() {
        vec![]
    } else {
        // SAFETY: body 非空分支；调用方契约保证 body 指向至少 body_len 字节、
        // 仅在本回调期间有效——立即 to_vec 拷贝、不持有。
        unsafe { std::slice::from_raw_parts(body, body_len).to_vec() }
    };
    // TODO: 包成 PortReply{corr=req_id, Ok(body)} 发给 engine_loop
}

unsafe fn ffi_str(ptr: *const u8, len: usize) -> Result<String, i32> {
    if ptr.is_null() {
        return Err(FFI_NULL);
    }
    // SAFETY: caller contract guarantees ptr points to len bytes for this call.
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| FFI_UTF8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::callbacks::HelixCallbacks;

    extern "C" fn noop_ws(_ctx: *mut std::ffi::c_void, _data: *const u8, _len: usize) -> i32 {
        0
    }

    extern "C" fn noop_http(
        _ctx: *mut std::ffi::c_void,
        _req_id: u64,
        _url: *const u8,
        _url_len: usize,
        _method: *const u8,
        _method_len: usize,
        _body: *const u8,
        _body_len: usize,
    ) {
    }

    extern "C" fn noop_event(_ctx: *mut std::ffi::c_void, _data: *const u8, _len: usize) {}

    extern "C" fn now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
        1
    }

    fn callbacks() -> HelixCallbacks {
        HelixCallbacks {
            send_ws_frame: Some(noop_ws),
            send_http: Some(noop_http),
            on_domain_event: Some(noop_event),
            now_ms: Some(now_ms),
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
            helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), callbacks())
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

        helix_destroy(handle.cast());
    }

    #[test]
    fn set_header_rejects_invalid_utf8() {
        let api = b"https://api.example.test";
        let ws = b"wss://ws.example.test/socket";
        let handle = unsafe {
            helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), callbacks())
        };
        assert!(!handle.is_null());

        let invalid = [0xff, 0xfe];
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
        assert_eq!(status, -2);

        helix_destroy(handle.cast());
    }
}
