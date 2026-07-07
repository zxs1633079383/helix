//! FFI API 端到端回调测试。
#![allow(clippy::undocumented_unsafe_blocks)]

use std::sync::Mutex as StdMutex;

use super::*;
use crate::vtable::CppHostVtable;

/// 录批回调收到的「解码后各条 DomainEvent 原文」。ctx 指向它。
type Recorder = StdMutex<Vec<Vec<u8>>>;

extern "C" fn recording_events(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    // SAFETY: ctx = &Recorder 裸指针（handle 存活期有效）；ptr/len 由 flush 传入，指向回调期有效 buffer。
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

    let mut got = false;
    for _ in 0..200 {
        if !recorder.lock().unwrap().is_empty() {
            got = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(got, "应在超时内收到至少一条领域事件批回调");

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

    unsafe { helix_destroy(handle) };
}

#[test]
fn create_applies_im_schema_before_mobile_query() {
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

    let query = b"im_query_dialog_list";
    let payload = b"{}";
    let mut out_ptr: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;
    let status = unsafe {
        helix_query(
            handle,
            query.as_ptr(),
            query.len(),
            payload.as_ptr(),
            payload.len(),
            &mut out_ptr,
            &mut out_len,
        )
    };
    assert_eq!(
        status, FFI_OK,
        "空库 dialog query 也应成功，不能因缺 IM 表 rc=-3"
    );
    let out = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let text = std::str::from_utf8(out).expect("query result utf8");
    assert!(text.contains("dialogList"), "query result = {text}");
    unsafe {
        helix_query_free(out_ptr, out_len);
        helix_destroy(handle);
    }
}
