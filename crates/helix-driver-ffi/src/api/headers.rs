use helix_core::tick::AppCommand;
use helix_core::AuthKind;

use super::command::send_command_tick;
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
    set_runtime_header(handle, &name, &value)
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
    remove_runtime_header(handle, &name)
}

fn set_runtime_header(handle: &HelixHandle, name: &str, value: &str) -> i32 {
    if name.eq_ignore_ascii_case("Authorization") {
        handle.auth.set_bot(name.to_string(), value.to_string());
        return FFI_OK;
    }

    if handle.headers.set_header_sync(name, value).is_err() {
        return FFI_INVALID;
    }
    if name.eq_ignore_ascii_case("cookieId") {
        handle.auth.set_session(name.to_string(), value.to_string());
        return sync_runtime_identity(handle, "auth_user_id", value);
    }
    if name.eq_ignore_ascii_case("companyId") {
        return sync_runtime_identity(handle, "company_id", value);
    }
    FFI_OK
}

fn remove_runtime_header(handle: &HelixHandle, name: &str) -> i32 {
    if name.eq_ignore_ascii_case("Authorization") {
        handle.auth.clear(AuthKind::Bot);
        return FFI_OK;
    }

    if handle.headers.remove_header_sync(name).is_err() {
        return FFI_INVALID;
    }
    if name.eq_ignore_ascii_case("cookieId") {
        handle.auth.clear(AuthKind::Session);
        return sync_runtime_identity(handle, "auth_user_id", "");
    }
    if name.eq_ignore_ascii_case("companyId") {
        return sync_runtime_identity(handle, "company_id", "");
    }
    FFI_OK
}

fn sync_runtime_identity(handle: &HelixHandle, field: &'static str, value: &str) -> i32 {
    let identity = match field {
        "auth_user_id" => serde_json::json!({ "auth_user_id": value }),
        "company_id" => serde_json::json!({ "company_id": value }),
        _ => return FFI_INVALID,
    };
    let payload = serde_json::to_vec(&identity)
        .expect("runtime identity payload serialization is infallible");
    send_command_tick(
        handle,
        AppCommand::new(helix_im::module::RUNTIME_IDENTITY_COMMAND, payload),
        None,
    )
}
