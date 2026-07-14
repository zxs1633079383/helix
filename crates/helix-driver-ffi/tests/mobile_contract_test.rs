use helix_driver_ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, helix_query, helix_query_free,
    HelixHandle, FFI_OK, FFI_UNIMPLEMENTED,
};
use helix_driver_ffi::batch::decode_batch;
use helix_driver_ffi::vtable::CppHostVtable;
use helix_driver_host::HostStorage;
use helix_im::IM_SCHEMA;
use std::sync::{Mutex, OnceLock};
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
    let _ = ensure_test_db();
    let api = b"https://api.example.test";
    let ws = b"wss://ws.example.test/socket";
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(record_batch),
        now_ms: Some(fixed_now_ms),
        ctx: recorder as *const Recorder as *mut std::ffi::c_void,
    };
    // SAFETY: input byte slices and callback vtable stay valid for the call.
    let handle = unsafe {
        helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
    };
    assert!(!handle.is_null(), "create 应成功");
    handle
}

fn ensure_test_db() -> &'static str {
    static DB_PATH: OnceLock<String> = OnceLock::new();
    DB_PATH.get_or_init(|| {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "helix-mobile-contract-test-{}.db",
            std::process::id()
        ));
        let db_path = path.to_string_lossy().into_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        runtime.block_on(async {
            let storage = HostStorage::open_sqlite_url(&db_path)
                .await
                .expect("open test db");
            for sql in IM_SCHEMA {
                storage.execute_raw(sql).await.expect("apply IM schema");
            }
        });
        std::env::set_var("HELIX_DB", &db_path);
        db_path
    })
}

fn seed_mobile_query_rows() {
    let _ = ensure_test_db();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime");
    runtime.block_on(async {
        let db_path = std::env::var("HELIX_DB").expect("HELIX_DB");
        let storage = HostStorage::open_sqlite_url(&db_path)
            .await
            .expect("open test db");
        storage
            .execute_raw(
                "INSERT OR REPLACE INTO channel \
                 (id, display_name, unread_count, mention_count, last_post, last_post_at, create_by) \
                 VALUES \
                 ('01hq0000000000000000000000', 'Mobile General', 7, 1, \
                  '{\"message\":\"seed last\",\"props\":{\"urgent\":\"u1\"}}', 1700000000010, 'mobile-test');
                 INSERT OR REPLACE INTO message \
                 (temporary_id, id, channel_id, user_id, type, message, create_at, update_at) \
                 VALUES \
                 ('tmp-mobile-seed', 'server-mobile-seed', '01hq0000000000000000000000', \
                  'user-mobile', 'TEXT', 'seed hello', 1700000000001, 1700000000002);",
            )
            .await
            .expect("seed mobile query rows");
    });
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
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "missing event {needle}"
        );
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

    let event = wait_for_event(&recorder, "im:post:sending");
    assert!(event.contains("tmp-mobile-1"));
    assert!(event.contains("01hq0000000000000000000000"));
    assert!(!event.contains("im:post:send-failed"));

    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn helix_capabilities_query_returns_mobile_contract_json() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let query_name = b"helix_capabilities";
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
    assert_eq!(status, FFI_OK);
    assert!(!out_ptr.is_null());
    assert!(out_len > 0);

    // SAFETY: out_ptr/out_len are owned by the successful helix_query result.
    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json["abi"], 1);
    assert_eq!(json["bus"], "im:__bus__");
    assert_eq!(json["features"]["pendingSendTimeoutUnsend"], true);
    assert!(json["commands"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "im_send"));
    let queries = json["queries"].as_array().unwrap();
    assert!(queries.iter().any(|v| v == "helix_capabilities"));
    assert!(queries.iter().any(|v| v == "helix_version"));
    assert!(queries.iter().any(|v| v == "im_query_dialog_list"));
    assert!(queries.iter().any(|v| v == "im_query_messages_by_channel"));
    assert!(json["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "im:post:send-failed"));

    // SAFETY: freeing the exact buffer returned by helix_query is required.
    unsafe { helix_query_free(out_ptr, out_len) };
    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn helix_version_query_returns_abi_json() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let query_name = b"helix_version";
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
    assert_eq!(status, FFI_OK);

    // SAFETY: out_ptr/out_len are owned by the successful helix_query result.
    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json, serde_json::json!({ "abi": 1 }));

    // SAFETY: freeing the exact buffer returned by helix_query is required.
    unsafe { helix_query_free(out_ptr, out_len) };
    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn local_dialog_list_query_returns_projection_json() {
    seed_mobile_query_rows();
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let query_name = b"im_query_dialog_list";
    let query_payload = br#"{"limit":10}"#;
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
    assert_eq!(status, FFI_OK);

    // SAFETY: out_ptr/out_len are owned by the successful helix_query result.
    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json["event"], "im:channels:projection");
    assert!(json["data"]["dialogList"].is_array());
    let dialog = json["data"]["dialogList"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "01hq0000000000000000000000")
        .expect("seeded dialog");
    assert_eq!(dialog["displayName"], "Mobile General");
    assert_eq!(dialog["unread"], 7);
    assert_eq!(dialog["mention"], true);
    assert_eq!(dialog["lastMessage"], "seed last");
    assert_eq!(dialog["urgent"], true);

    // SAFETY: freeing the exact buffer returned by helix_query is required.
    unsafe { helix_query_free(out_ptr, out_len) };
    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn local_message_query_returns_projection_json() {
    seed_mobile_query_rows();
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let query_name = b"im_query_messages_by_channel";
    let query_payload = br#"{"channelId":"01hq0000000000000000000000","limit":25}"#;
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
    assert_eq!(status, FFI_OK);

    // SAFETY: out_ptr/out_len are owned by the successful helix_query result.
    let bytes = unsafe { std::slice::from_raw_parts(out_ptr, out_len) };
    let json: serde_json::Value = serde_json::from_slice(bytes).unwrap();
    assert_eq!(json["event"], "im:messages:query_result");
    assert_eq!(json["data"]["channel_id"], "01hq0000000000000000000000");
    let messages = json["data"]["messages"].as_array().unwrap();
    let message = messages
        .iter()
        .find(|item| item["temporaryId"] == "tmp-mobile-seed")
        .expect("seeded message");
    assert_eq!(message["msgId"], "server-mobile-seed");
    assert_eq!(message["channelId"], "01hq0000000000000000000000");
    assert_eq!(message["text"], "seed hello");
    assert_eq!(message["type"], "TEXT");
    assert_eq!(message["createAt"], 1700000000001i64);

    // SAFETY: freeing the exact buffer returned by helix_query is required.
    unsafe { helix_query_free(out_ptr, out_len) };
    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}

#[test]
fn unknown_query_returns_unimplemented_and_clears_out_params() {
    let recorder = Recorder::new(Vec::new());
    let handle = create_recording_handle(&recorder);
    let query_name = b"not_a_query";
    let query_payload = b"{}";
    let mut out_ptr: *mut u8 = std::ptr::dangling_mut::<u8>();
    let mut out_len: usize = 999;

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
    assert!(out_ptr.is_null());
    assert_eq!(out_len, 0);

    // SAFETY: handle was created by this test and not yet destroyed.
    unsafe { helix_destroy(handle) };
}
