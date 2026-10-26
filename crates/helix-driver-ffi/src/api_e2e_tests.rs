//! FFI API 端到端回调测试。
#![allow(clippy::undocumented_unsafe_blocks)]

use std::sync::Mutex as StdMutex;

use super::*;
use crate::vtable::CppHostVtable;

use crate as ffi;

#[path = "../tests/support/owned_channel_fixture.rs"]
mod owned_channel_fixture;

use owned_channel_fixture::{fixed_now_ms, recording_events, Recorder};

/// 回归锚: M3-F4 —— 命令进 → 领域事件经批回调出（端到端打通证据）。
///
/// 不设 HELIX_DB：每个 create 走 `ffi_db_url` 的进程+计数临时库 fallback（各 handle 独立库，
/// 不与并发测试撞库、不污染共享 env）。
#[test]
fn command_emits_domain_event_through_batch_callback() {
    owned_channel_fixture::assert_owned_channel_send();
}

#[test]
/// 空库异步 dialog query 必须在 schema 初始化后返回唯一 typed read Result。
fn create_applies_im_schema_before_async_business_query() {
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
    assert!(!handle.is_null(), "create 应成功并应用 IM schema");

    for (name, value) in [
        (b"cookieId".as_slice(), b"viewer-1".as_slice()),
        (b"companyId".as_slice(), b"team-1".as_slice()),
    ] {
        let status = unsafe {
            helix_set_header(
                handle,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            )
        };
        assert_eq!(
            status, FFI_OK,
            "完整 viewer scope 必须先进入共享 command pump"
        );
    }

    let query = b"im_query_dialog_list";
    let payload = b"{}";
    let status = unsafe {
        helix_command(
            handle,
            query.as_ptr(),
            query.len(),
            payload.as_ptr(),
            payload.len(),
        )
    };
    assert_eq!(status, FFI_OK, "异步业务 query 应进入共享 command pump");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut got_projection = false;
    while std::time::Instant::now() < deadline {
        got_projection = recorder.lock().unwrap().iter().any(|event| {
            std::str::from_utf8(event)
                .map(|text| text.contains("im:read:result") && text.contains("\"items\":[]"))
                .unwrap_or(false)
        });
        if got_projection {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        got_projection,
        "空库异步 dialog query 应产出 typed empty read Result"
    );

    unsafe { helix_destroy(handle) };
}
