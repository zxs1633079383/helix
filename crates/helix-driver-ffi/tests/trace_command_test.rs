use std::fs;
use std::path::PathBuf;

use helix_driver_ffi::api::{
    helix_command_with_trace, helix_create_with_network, helix_destroy, HelixHandle, FFI_INVALID,
    FFI_OK,
};
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::trace::TRACE_CARRIER_JSON_MAX_BYTES;

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

fn create_handle() -> *mut HelixHandle {
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
fn generated_headers_export_command_with_trace() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let header_c = fs::read_to_string(crate_dir.join("helix.h")).expect("read helix.h");
    let header_cpp = fs::read_to_string(crate_dir.join("helix.hpp")).expect("read helix.hpp");

    assert!(
        header_c.contains("helix_command_with_trace"),
        "helix.h should export helix_command_with_trace"
    );
    assert!(
        header_cpp.contains("helix_command_with_trace"),
        "helix.hpp should export helix_command_with_trace"
    );
}

#[test]
fn trace_command_accepts_valid_sidecar_and_preserves_payload_shape() {
    let handle = create_handle();
    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-1","text":"hi"}"#;
    let trace = br#"{"traceparent":"00-00000000000000000000000000000001-0000000000000002-01","baggage":"client=test"}"#;

    // SAFETY: handle is live and all byte pointers are valid for this call.
    let status = unsafe {
        helix_command_with_trace(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            trace.as_ptr(),
            trace.len(),
        )
    };
    assert_eq!(status, FFI_OK);

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn trace_command_rejects_malformed_sidecar() {
    let handle = create_handle();
    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-1","text":"hi"}"#;
    let trace = br#"{"traceparent":"00-00000000000000000000000000000000-0000000000000002-01"}"#;

    // SAFETY: handle is live and all byte pointers are valid for this call.
    let status = unsafe {
        helix_command_with_trace(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            trace.as_ptr(),
            trace.len(),
        )
    };
    assert_eq!(status, FFI_INVALID);

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn trace_command_rejects_oversized_sidecar_before_copy() {
    let handle = create_handle();
    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-1","text":"hi"}"#;
    let trace = vec![b' '; TRACE_CARRIER_JSON_MAX_BYTES + 1];

    // SAFETY: handle is live and all byte pointers are valid for this call.
    let status = unsafe {
        helix_command_with_trace(
            handle,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
            trace.as_ptr(),
            trace.len(),
        )
    };
    assert_eq!(status, FFI_INVALID);

    // SAFETY: handle was created by this test and has not been destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn trace_command_source_starts_mobile_ffi_boundary_span() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(crate_dir.join("src/api.rs")).expect("read helix-driver-ffi src/api.rs");

    assert!(
        source.contains("\"mobile.ffi.command\""),
        "helix_command_with_trace should start the mobile.ffi.command span"
    );
    assert!(
        source.contains("TraceDirection::Inbound"),
        "mobile FFI trace boundary should be marked inbound"
    );
    assert!(
        source.contains("HostOtelRuntime::from_env(\"helix-driver-ffi\")"),
        "FFI handle should load an env-configurable host OTel runtime"
    );
}
