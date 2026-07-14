use helix_core::AuthKind;

use super::handle::HelixHandle;
use super::shared::{ffi_str, FFI_INVALID, FFI_NULL, FFI_OK};

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
