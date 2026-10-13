use helix_driver_ffi as ffi;
use helix_driver_ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, helix_query, helix_query_free,
    helix_remove_header, helix_set_header, FFI_NULL, FFI_OK, FFI_UNIMPLEMENTED, FFI_UTF8,
};
use helix_driver_ffi::vtable::CppHostVtable;

#[path = "support/owned_channel_fixture.rs"]
mod owned_channel_fixture;

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

fn create_handle() -> *mut helix_driver_ffi::api::HelixHandle {
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    // SAFETY: byte pointers and lengths are valid for the call duration; vtable
    // callbacks are static test functions.
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
    handle
}

#[test]
fn set_header_rejects_null_handle() {
    let name = b"authorization";
    let value = b"Bearer token";
    // SAFETY: null handle is intentionally passed to verify the API error path;
    // name/value pointers are valid for the call duration.
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
    let handle = create_handle();
    let name = b"authorization";
    let value = b"Bearer token";
    // SAFETY: handle is live and owned by this test; name/value pointers are
    // valid for the call duration.
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

    // SAFETY: handle is live and owned by this test; name pointer is valid for
    // the call duration.
    let remove_status = unsafe { helix_remove_header(handle, name.as_ptr(), name.len()) };
    assert_eq!(remove_status, 0);

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn set_header_rejects_invalid_utf8() {
    let handle = create_handle();
    let invalid = [0xff_u8, 0xfe];
    let value = b"Bearer token";
    // SAFETY: handle is live; invalid bytes intentionally exercise UTF-8
    // validation, and value pointer is valid for the call duration.
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

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn command_rejects_null_handle() {
    let name = b"test_cmd";
    let payload = b"{}";
    // SAFETY: null handle is intentionally passed to verify the API error path;
    // command and payload pointers are valid for the call duration.
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
    let handle = create_handle();
    let name = b"send_message";
    let payload = b"{\"text\":\"hello\"}";
    // SAFETY: handle is live and owned by this test; command/payload pointers
    // are valid for the call duration.
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

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn query_returns_unimplemented() {
    let handle = create_handle();
    let query_name = b"get_channels";
    let query_payload = b"{}";
    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    // SAFETY: handle is live; query pointers and out-params are valid for the
    // call duration.
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

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn query_free_handles_null() {
    // SAFETY: null pointer is explicitly allowed as a no-op by helix_query_free.
    unsafe { helix_query_free(std::ptr::null_mut(), 0) };
}

#[test]
fn command_emits_domain_event_through_batch_callback() {
    owned_channel_fixture::assert_owned_channel_send();
}

#[test]
fn pump_lifecycle_create_then_destroy_drains_cleanly() {
    let handle = create_handle();
    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}
