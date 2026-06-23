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

// ── 端到端：命令 → step → Emit → 批回调（真 extern "C" vtable 录回调 bytes，证 helix_command →
// pump step → Effect::Emit → FfiEventSink 攒批 → on_domain_events_batch 打通，decode_batch 还原断言）──

use std::sync::Mutex as StdMutex;

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
