//! Shared test-only public FFI owned-channel hydration and send scenario.

use super::ffi;
use ffi::api::{
    helix_command, helix_create_with_network, helix_destroy, helix_set_header, HelixHandle, FFI_OK,
};
use ffi::vtable::CppHostVtable;
use std::sync::Mutex as StdMutex;

const SERVER_POST_ID: &str = "01hq0000000000000000000001";

/// 录批回调收到的「解码后各条 DomainEvent 原文」。ctx 指向它。
pub(super) type Recorder = StdMutex<Vec<Vec<u8>>>;

pub(super) extern "C" fn recording_events(ctx: *mut std::ffi::c_void, ptr: *const u8, len: usize) {
    // SAFETY: ctx = &Recorder 裸指针（handle 存活期有效）；ptr/len 由 flush 传入，指向回调期有效 buffer。
    let rec = unsafe { &*(ctx as *const Recorder) };
    // SAFETY: callback-local ptr/len stay valid until this callback returns.
    let buf = unsafe { std::slice::from_raw_parts(ptr, len) };
    if let Some(events) = ffi::batch::decode_batch(buf) {
        let mut g = rec.lock().unwrap();
        for e in events {
            g.push(e.to_vec());
        }
    }
}

pub(super) extern "C" fn fixed_now_ms(_ctx: *mut std::ffi::c_void) -> u64 {
    1_700_000_000_000
}

/// The callback context must outlive the pump even when an assertion unwinds.
struct TestHandle(*mut HelixHandle);

impl Drop for TestHandle {
    fn drop(&mut self) {
        // SAFETY: this guard owns the non-null handle and destroys it exactly once.
        unsafe { helix_destroy(self.0) };
    }
}

#[derive(Debug)]
struct FixtureRequest {
    method: String,
    path: String,
    body: serde_json::Value,
}

struct HttpFixture {
    api: String,
    requests: std::sync::mpsc::Receiver<FixtureRequest>,
    stopping: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

struct WsFixture {
    url: String,
    echo_tx: tokio::sync::mpsc::UnboundedSender<String>,
    stopping: Option<tokio::sync::oneshot::Sender<()>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl WsFixture {
    fn new(channel_id: &str) -> Self {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::{accept_async, tungstenite::Message};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let (echo_tx, mut echo_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel();
        let channel_id = channel_id.to_owned();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let accepted = tokio::select! {
                    result = listener.accept() => result.ok(),
                    _ = &mut stop_rx => None,
                };
                let Some((stream, _)) = accepted else { return };
                let mut websocket = accept_async(stream).await.unwrap();
                let temporary_id = tokio::select! {
                    temporary_id = echo_rx.recv() => temporary_id,
                    _ = &mut stop_rx => None,
                };
                let Some(temporary_id) = temporary_id else {
                    return;
                };
                let frame = serde_json::json!({
                    "v": 1,
                    "action": "post",
                    "data": {
                        "id": SERVER_POST_ID,
                        "temporaryId": temporary_id,
                        "channelId": channel_id,
                        "event_seq": 1,
                        "userId": "viewer-1",
                        "teamId": "team-1",
                        "type": "TEXT",
                        "message": "hi",
                        "simpleMessage": "hi",
                        "props": {},
                        "viewers": ["all"],
                        "createAt": 1_700_000_000_000_u64,
                        "updateAt": 1_700_000_000_000_u64,
                    },
                });
                websocket
                    .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
                    .await
                    .unwrap();
                tokio::select! {
                    _ = &mut stop_rx => {},
                    _ = websocket.next() => {},
                }
            });
        });
        Self {
            url: format!("ws://{addr}/api/v4/websocket"),
            echo_tx,
            stopping: Some(stop_tx),
            worker: Some(worker),
        }
    }

    fn send_echo(&self, temporary_id: &str) {
        self.echo_tx
            .send(temporary_id.to_owned())
            .expect("WS fixture handshake worker must be alive");
    }

    fn finish(&mut self) {
        if let Some(stopping) = self.stopping.take() {
            stopping.send(()).ok();
        }
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("WS fixture worker failed");
    }
}

impl Drop for WsFixture {
    fn drop(&mut self) {
        if let Some(stopping) = self.stopping.take() {
            stopping.send(()).ok();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl HttpFixture {
    fn new(channel_id: &str) -> Self {
        use std::io::{Read, Write};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}/api/cses", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stopping = std::sync::Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let (request_tx, requests) = std::sync::mpsc::channel();
        // 完整频道携带 type；官方稀疏快照围栏只更新既有行，不能建立首次归属。
        let increment = serde_json::json!({ "data": {
            "id": channel_id, "type": "O", "teamId": "team-1", "displayName": "scope-hydration",
            "lastEventSeq": 0, "unreadCount": 0, "needSync": false, "memberCount": 1,
            "members": [{ "userId": "viewer-1", "teamId": "team-1", "role": "MEMBER" }]
        }});
        let worker = std::thread::spawn(move || {
            while !worker_stopping.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("HTTP fixture accept failed: {error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0; 4096];
                let header_end = loop {
                    if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        break index + 4;
                    }
                    let read = stream.read(&mut chunk).unwrap();
                    assert!(
                        read > 0 && bytes.len() + read <= 65_536,
                        "invalid HTTP fixture request"
                    );
                    bytes.extend_from_slice(&chunk[..read]);
                };
                let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
                let mut request_line = header.lines().next().unwrap().split_whitespace();
                let method = request_line.next().unwrap().to_owned();
                let path = request_line.next().unwrap().to_owned();
                let content_length: usize = header
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse().unwrap())
                    .expect("fixture POST must declare its body length");
                assert!(header_end + content_length <= 65_536);
                while bytes.len() < header_end + content_length {
                    let read = stream.read(&mut chunk).unwrap();
                    assert!(read > 0, "HTTP fixture body was truncated");
                    bytes.extend_from_slice(&chunk[..read]);
                }
                let body: serde_json::Value =
                    serde_json::from_slice(&bytes[header_end..header_end + content_length])
                        .unwrap();
                let response = match path.as_str() {
                    "/api/cses/channel/load/incrementByChannelId" => increment.clone(),
                    "/api/cses/posts/create" => serde_json::json!({
                        "status": "SUCCESS", "data": { "id": SERVER_POST_ID }
                    }),
                    "/api/cses/post/clientAck" => {
                        serde_json::json!({ "status": "SUCCESS", "data": {} })
                    }
                    _ => panic!("unexpected HTTP fixture path: {path}"),
                };
                request_tx
                    .send(FixtureRequest { method, path, body })
                    .unwrap();
                let response = serde_json::to_vec(&response).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                ).unwrap();
                stream.write_all(&response).unwrap();
            }
        });
        Self {
            api,
            requests,
            stopping,
            worker: Some(worker),
        }
    }

    fn finish(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        self.worker
            .take()
            .unwrap()
            .join()
            .expect("HTTP fixture worker failed");
    }
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.stopping
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(worker) = self.worker.take() {
            // Preserve an assertion panic while still joining the bounded fixture worker.
            let _ = worker.join();
        }
    }
}

fn wait_for_event(
    recorder: &Recorder,
    start: usize,
    matches: impl Fn(&serde_json::Value) -> bool,
) -> Option<serde_json::Value> {
    // Keep the original send callback bound: 200 attempts with a 10 ms interval.
    for _ in 0..200 {
        let events = recorder.lock().unwrap()[start..].to_vec();
        if let Some(event) = events.iter().find_map(|bytes| {
            let event = serde_json::from_slice(bytes).expect("domain event must be JSON");
            matches(&event).then_some(event)
        }) {
            return Some(event);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

pub(super) fn assert_owned_channel_send() {
    const CHANNEL: &str = "01hq0000000000000000000000";
    let recorder: Box<Recorder> = Box::new(StdMutex::new(Vec::new()));
    let ctx = &*recorder as *const Recorder as *mut std::ffi::c_void;
    let vtable = CppHostVtable {
        on_domain_events_batch: Some(recording_events),
        now_ms: Some(fixed_now_ms),
        ctx,
    };

    let mut fixture = HttpFixture::new(CHANNEL);
    let mut ws_fixture = WsFixture::new(CHANNEL);
    let api = fixture.api.as_bytes();
    let ws = ws_fixture.url.as_bytes();
    // SAFETY: endpoint bytes, callbacks and recorder ctx remain valid through guard destruction.
    let handle = unsafe {
        helix_create_with_network(api.as_ptr(), api.len(), ws.as_ptr(), ws.len(), vtable)
    };
    assert!(
        !handle.is_null(),
        "create 应成功（storage 开库 + pump 起线程）"
    );
    let handle = TestHandle(handle);

    for (name, value) in [
        (b"cookieId".as_slice(), b"viewer-1".as_slice()),
        (b"companyId".as_slice(), b"team-1".as_slice()),
    ] {
        // SAFETY: the guard owns a live handle; header slices are valid for this call.
        let status = unsafe {
            helix_set_header(
                handle.0,
                name.as_ptr(),
                name.len(),
                value.as_ptr(),
                value.len(),
            )
        };
        assert_eq!(
            status, FFI_OK,
            "viewer identity must enter the shared command pump"
        );
    }
    let seed_name = b"im_ensure_channel_loaded";
    let seed_payload =
        br#"{"channel_id":"01hq0000000000000000000000","req_id":"seed-owned-channel"}"#;
    // SAFETY: the guard owns a live handle; command/payload slices are valid for this call.
    let status = unsafe {
        helix_command(
            handle.0,
            seed_name.as_ptr(),
            seed_name.len(),
            seed_payload.as_ptr(),
            seed_payload.len(),
        )
    };
    assert_eq!(
        status, FFI_OK,
        "owned channel hydration must enter the public pump"
    );
    let seed_request = fixture
        .requests
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("public hydration must make a real HTTP request");
    assert_eq!(seed_request.method, "POST");
    assert_eq!(
        seed_request.path,
        "/api/cses/channel/load/incrementByChannelId"
    );
    assert_eq!(
        seed_request.body,
        serde_json::json!({ "channelId": CHANNEL })
    );

    // This terminal is published only after channel/member/message/cursor durable readback.
    let hydrated = wait_for_event(&recorder, 0, |event| {
        event["event"] == "im:read:result" && event["data"]["req_id"] == "seed-owned-channel"
    })
    .expect("owned channel hydration must finish through the batch callback");
    assert!(
        hydrated["data"].get("error").is_none(),
        "hydration failed: {hydrated}"
    );
    let body = &hydrated["data"]["body"];
    assert_eq!(body["completion"], "hydrated");
    assert_eq!(body["channelId"], CHANNEL);
    assert_eq!(body["channel"]["id"], CHANNEL);
    assert_eq!(body["channel"]["type"], "O");
    assert_eq!(body["channel"]["teamId"], "team-1");
    assert_eq!(body["channel"]["userId"], "viewer-1");
    assert_eq!(body["member"]["channel_id"], CHANNEL);
    assert_eq!(body["member"]["team_id"], "team-1");
    assert_eq!(body["member"]["user_id"], "viewer-1");
    assert_eq!(body["member"]["role"], "MEMBER");
    assert_eq!(body["messages"], serde_json::json!([]));
    assert_eq!(body["cursor"], 0);
    let seed_events = recorder.lock().unwrap().clone();
    assert_eq!(
        seed_events
            .iter()
            .filter(|bytes| {
                let event: serde_json::Value = serde_json::from_slice(bytes).unwrap();
                event["event"] == "im:read:result"
                    && event["data"]["req_id"] == "seed-owned-channel"
            })
            .count(),
        1,
        "hydration must emit exactly one correlated terminal"
    );
    let callback_start = seed_events.len();

    let name = b"im_send_message";
    let payload =
        br#"{"channel_id":"01hq0000000000000000000000","temporary_id":"tmp-1","text":"hi"}"#;
    // SAFETY: the guard owns a live handle; command/payload slices are valid for this call.
    let status = unsafe {
        helix_command(
            handle.0,
            name.as_ptr(),
            name.len(),
            payload.as_ptr(),
            payload.len(),
        )
    };
    assert_eq!(status, FFI_OK, "命令入队应成功");

    let sending = wait_for_event(&recorder, callback_start, |event| {
        event["event"] == "im:post:sending"
    })
    .expect("original send must emit im:post:sending after its optimistic persist");
    let data = &sending["data"];
    assert_eq!(data["channelId"], CHANNEL);
    assert_eq!(data["userId"], "viewer-1");
    assert_eq!(data["message"], "hi");
    assert_eq!(data["type"], "TEXT");
    assert_eq!(data["sendStatus"], "sending");
    assert_eq!(data["createAt"], fixed_now_ms(std::ptr::null_mut()));
    let temporary_id = data["temporaryId"]
        .as_str()
        .expect("sending has a generated temporaryId");
    assert!(!temporary_id.is_empty());
    assert_ne!(
        temporary_id, "tmp-1",
        "legacy temporary_id is not send authority"
    );

    let post_request = fixture
        .requests
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("original send must make a real posts/create request");
    assert_eq!(post_request.method, "POST");
    assert_eq!(post_request.path, "/api/cses/posts/create");
    assert_eq!(post_request.body["channelId"], CHANNEL);
    assert_eq!(post_request.body["userId"], "viewer-1");
    assert_eq!(post_request.body["teamId"], "team-1");
    assert_eq!(post_request.body["temporaryId"], temporary_id);
    assert_eq!(post_request.body["message"], data["message"]);
    assert_eq!(post_request.body["type"], data["type"]);
    assert_eq!(post_request.body["createAt"], data["createAt"]);
    ws_fixture.send_echo(temporary_id);
    let sent = wait_for_event(&recorder, callback_start, |event| {
        event["event"] == "im:post:sent"
            && event["data"]["temporaryId"] == temporary_id
            && event["data"]["sendStatus"] == "sent"
    })
    .expect("real localhost WS echo must publish im:post:sent");
    assert_eq!(sent["data"]["channelId"], CHANNEL);
    assert_eq!(sent["data"]["id"], SERVER_POST_ID);
    assert_eq!(sent["data"]["serverId"], SERVER_POST_ID);
    let sent_count = recorder.lock().unwrap()[callback_start..]
        .iter()
        .filter(|bytes| {
            let event: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            event["event"] == "im:post:sent" && event["data"]["temporaryId"] == temporary_id
        })
        .count();
    assert_eq!(sent_count, 1, "durable send terminal is unique");
    println!("FFI durable hydration: {hydrated}");
    println!("FFI post-seed sending callback: {sending}");
    println!(
        "FFI correlated real posts/create body: {}",
        post_request.body
    );
    println!("FFI durable send terminal: {sent}");
    drop(handle);
    fixture.finish();
    ws_fixture.finish();
}
