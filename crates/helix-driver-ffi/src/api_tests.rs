//! `api.rs` 单测外提 sibling（structure-gate「测试外提」范本）。`#[path]` 子模块：`super` = `api`。
//! 测试内调本 crate FFI 导出（入参测试栈构造的有效 ptr/len），逐块 SAFETY 冗余 → 文件级豁免。
#![allow(clippy::undocumented_unsafe_blocks)]

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
fn command_maps_invalid_public_payload_to_ffi_invalid() {
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

    let name = b"im_send";
    let invalid_payload = b"{not-json";
    let status = unsafe {
        helix_command(
            handle,
            name.as_ptr(),
            name.len(),
            invalid_payload.as_ptr(),
            invalid_payload.len(),
        )
    };
    assert_eq!(status, FFI_INVALID);

    unsafe { helix_destroy(handle) };
}

#[test]
fn command_with_trace_accepts_valid_w3c_sidecar() {
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
    let trace_json =
        br#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01","baggage":"tenant=im"}"#;
    let status = unsafe {
        helix_command_with_trace(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            trace_json.as_ptr(),
            trace_json.len(),
        )
    };
    assert_eq!(status, FFI_OK);

    unsafe { helix_destroy(handle) };
}

#[test]
fn command_with_trace_rejects_invalid_trace_json() {
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
    let trace_json = b"{not-json";
    let status = unsafe {
        helix_command_with_trace(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            trace_json.as_ptr(),
            trace_json.len(),
        )
    };
    assert_eq!(status, FFI_INVALID);

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
