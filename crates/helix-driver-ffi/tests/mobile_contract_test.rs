use helix_driver_ffi::api::{FFI_OK, HelixHandle, helix_command, helix_create_with_network, helix_destroy};
use helix_driver_ffi::batch::decode_batch;
use helix_driver_ffi::vtable::CppHostVtable;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Recorder = Mutex<Vec<Vec<u8>>>;

extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

extern "C" fn record_batch(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    // SAFETY: ctx points at the Recorder owned by this test for the entire
    // handle lifetime.
    let rec = unsafe { &*(ctx as *const Recorder) };
    // SAFETY: ptr/len are valid only for the callback; we decode immediately.
    let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Some(events) = decode_batch(buf) {
        let mut guard = rec.lock().unwrap();
        for event in events {
            guard.push(event.to_vec());
        }
    }
}

fn create_recording_handle(recorder: &Recorder) -> *mut HelixHandle {
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(record_batch),
        now_ms: Some(fixed_now_ms),
        ctx: recorder as *const Recorder as *mut std::ffi::c_void,
    };
    // SAFETY: input byte slices and callback vtable stay valid for the call.
    let handle =
        unsafe { helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable) };
    assert!(!handle.is_null(), "create 应成功");
    handle
}

fn wait_for_event(recorder: &Recorder, needle: &str) -> String {
    let start = Instant::now();
    loop {
        if let Some(found) = recorder
            .lock()
            .unwrap()
            .iter()
            .filter_map(|bytes| std::str::from_utf8(bytes).ok())
            .find(|text| text.contains(needle))
            .map(str::to_owned)
        {
            return found;
        }
        assert!(start.elapsed() < Duration::from_secs(3), "missing event {needle}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn mobile_im_send_alias_accepts_camelcase_payload_and_emits_sending() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let name = b"im_send";
    let payload =
        br#"{"channelId":"01hq0000000000000000000000","temporaryId":"tmp-mobile-1","text":"hi","type":"TEXT"}"#;

    // SAFETY: handle is live and the input buffers stay valid for this call.
    let status =
        unsafe { helix_command(handle, name.as_ptr(), name.len(), payload.as_ptr(), payload.len()) };
    assert_eq!(status, FFI_OK);

    let event = wait_for_event(&recorder, "im:post:sending");
    assert!(event.contains("tmp-mobile-1"));
    assert!(event.contains("01hq0000000000000000000000"));
    assert!(!event.contains("im:post:send-failed"));

    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}
