//! LOCAL_INTEGRATION：真实 host engine + SQLite + streaming range PUT。
//!
//! Java/OSS/Go 在本地 TCP 端口提供最小协议替身；上传本身走生产
//! `SharedFileUploader`，不会把 101 MiB fixture 聚合到测试进程内存。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpResponse, Row, ScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::{Clock, EventSink, FrameSender, Storage};
use helix_core::{ExecutionShell, PortError, Tick};
use helix_driver_host::{
    run_engine_loop, AuthTokenRegistry, BatchSink, CrossCuttingHttp, EngineDeps, HostNetworkConfig,
    HostStorage, NoopMetricSink, SharedFileUploader, SharedHttpClient, TraceHooks,
    TransportRegistration,
};
use helix_im::module::ImConfig;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const AUTH_USER_ID: &str = "usfixx00000000000000000001";
const AUTH_DEVICE_ID: &str = "device-multipart-pipeline";
const TEMPORARY_ID: &str = "tmp-multipart-pipeline-101m";
const FILE_SIZE: u64 = 101 * 1024 * 1024;
const PART_SIZE: u64 = 8 * 1024 * 1024;
const PART_COUNT: u32 = 13;
const FILE_SHA256: &str = "f1753a034bbd26e9458921b121c32d4750b9f7e5ef8adbbb337dcc96283ef40a";

#[derive(Debug)]
struct LocalRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone)]
struct JavaRequestRecord {
    path: String,
    body: Value,
}

#[derive(Debug, Clone)]
struct RemotePart {
    size: u64,
    etag: String,
}

#[derive(Debug, Clone)]
struct PartsSnapshot {
    status: String,
    parts: BTreeMap<u32, RemotePart>,
}

#[derive(Debug, Default)]
struct JavaAudit {
    requests: Vec<JavaRequestRecord>,
    parts_snapshots: Vec<PartsSnapshot>,
    complete_seen: bool,
    abort_seen: bool,
}

#[derive(Debug, Clone)]
struct OssPutRecord {
    part_number: u32,
    size: u64,
    etag: String,
    target: String,
    headers: BTreeMap<String, String>,
}

#[derive(Default)]
struct RecordingEvents(Mutex<Vec<Value>>);

impl EventSink for RecordingEvents {
    fn emit(&self, event: DomainEventBytes) {
        if let Ok(value) = serde_json::from_slice(&event.0) {
            self.0.lock().expect("event recorder lock").push(value);
        }
    }
}

impl BatchSink for RecordingEvents {}

#[derive(Clone, Copy)]
struct FixedClock;

impl Clock for FixedClock {
    fn now_ms(&self) -> u64 {
        1_725_000_000_000
    }
}

struct UnusedFrameSender;

#[async_trait::async_trait]
impl FrameSender for UnusedFrameSender {
    async fn send(&self, _: Bytes) -> Result<(), PortError> {
        Ok(())
    }
}

/// Read just the HTTP header block. Reading one byte at a time keeps the body
/// out of this buffer, which lets the OSS fixture consume each 8 MiB range
/// with a fixed 64 KiB scratch buffer rather than collecting the whole file.
async fn read_headers(stream: &mut TcpStream) -> (String, BTreeMap<String, String>) {
    let mut raw = Vec::with_capacity(512);
    loop {
        let mut byte = [0_u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .expect("read local HTTP header");
        raw.push(byte[0]);
        if raw.ends_with(b"\r\n\r\n") {
            break;
        }
        assert!(raw.len() <= 64 * 1024, "local HTTP header exceeded bound");
    }
    let text = String::from_utf8(raw).expect("local HTTP header utf8");
    let mut lines = text.lines();
    let request_line = lines.next().expect("local HTTP request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().expect("local HTTP method").to_string();
    let target = request_parts.next().expect("local HTTP target").to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    (format!("{method} {target}"), headers)
}

async fn read_small_request(stream: &mut TcpStream) -> LocalRequest {
    let (request_line, headers) = read_headers(stream).await;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().expect("request method").to_string();
    let target = request_parts.next().expect("request target").to_string();
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0_u8; content_length];
    stream
        .read_exact(&mut body)
        .await
        .expect("read local JSON body");
    LocalRequest {
        method,
        target,
        headers,
        body,
    }
}

async fn write_json_response(stream: &mut TcpStream, value: &Value) {
    let body = serde_json::to_vec(value).expect("serialize local JSON response");
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .expect("write local response headers");
    stream
        .write_all(&body)
        .await
        .expect("write local response body");
}

fn expected_part_size(part_number: u32) -> u64 {
    assert!((1..=PART_COUNT).contains(&part_number));
    let offset = (u64::from(part_number) - 1) * PART_SIZE;
    FILE_SIZE.saturating_sub(offset).min(PART_SIZE)
}

fn common_java_assertions(request: &LocalRequest) {
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.headers.get("cookieid").map(String::as_str),
        Some(AUTH_USER_ID),
        "Java receives platform session credential"
    );
    assert_eq!(
        request.headers.get("deviceid").map(String::as_str),
        Some(AUTH_DEVICE_ID),
        "Java receives platform device identity"
    );
    assert!(
        !request.headers.contains_key("x-auth-kind"),
        "auth intent must be stripped before Java"
    );
}

fn assert_common_body(body: &Value, upload_id: Option<&str>) {
    assert_eq!(
        body["clientUploadId"].as_str().map(str::is_empty),
        Some(false)
    );
    assert_eq!(body["bucket"], "attachment");
    assert_eq!(body["contentType"], "application/octet-stream");
    assert_eq!(body["size"], FILE_SIZE);
    assert_eq!(body["sha256"], FILE_SHA256);
    if let Some(upload_id) = upload_id {
        assert_eq!(body["uploadId"], upload_id);
    }
}

async fn run_java_server(
    listener: TcpListener,
    oss_address: std::net::SocketAddr,
    remote_parts: Arc<Mutex<BTreeMap<u32, RemotePart>>>,
    audit: Arc<Mutex<JavaAudit>>,
) {
    loop {
        let (mut stream, _) = listener.accept().await.expect("accept Java request");
        let request = read_small_request(&mut stream).await;
        common_java_assertions(&request);
        let path = request
            .target
            .split('?')
            .next()
            .expect("Java path")
            .to_string();
        let body: Value = serde_json::from_slice(&request.body).expect("parse Java JSON");
        let upload_id = "upload-multipart-101m";
        let file_id = "file-multipart-101m";
        let object_key = format!("media/{file_id}");

        let response = match path.as_str() {
            "/java/oss/multipart/init" => {
                assert_common_body(&body, None);
                assert_eq!(body["fileName"], "multipart-101m.bin");
                json!({
                    "isSuccess": true,
                    "protocolVersion": 1,
                    "status": "uploading",
                    "uploadId": upload_id,
                    "fileId": file_id,
                    "bucket": "attachment",
                    "objectKey": object_key,
                    "uri": format!("media/{file_id}"),
                    "partSize": PART_SIZE,
                    "size": FILE_SIZE,
                    "sha256": FILE_SHA256
                })
            }
            "/java/oss/multipart/parts" => {
                assert_common_body(&body, Some(upload_id));
                let parts = remote_parts.lock().expect("remote parts lock").clone();
                let status = if parts.len() == PART_COUNT as usize {
                    "completed"
                } else {
                    "uploading"
                };
                audit
                    .lock()
                    .expect("Java audit lock")
                    .parts_snapshots
                    .push(PartsSnapshot {
                        status: status.to_string(),
                        parts: parts.clone(),
                    });
                let response_parts = parts
                    .iter()
                    .map(|(part_number, part)| {
                        json!({
                            "partNumber": part_number,
                            "size": part.size,
                            "etag": part.etag
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "isSuccess": true,
                    "uploadId": upload_id,
                    "status": status,
                    "parts": response_parts
                })
            }
            "/java/oss/multipart/sign" => {
                assert_common_body(&body, Some(upload_id));
                let part_numbers = body["partNumbers"]
                    .as_array()
                    .expect("sign partNumbers array")
                    .iter()
                    .map(|value| value.as_u64().expect("sign part number") as u32)
                    .collect::<Vec<_>>();
                assert!((1..=8).contains(&part_numbers.len()));
                let unique = part_numbers.iter().copied().collect::<BTreeSet<_>>();
                assert_eq!(unique.len(), part_numbers.len());
                assert!(unique.iter().all(|part| (1..=PART_COUNT).contains(part)));
                let signed = part_numbers
                    .iter()
                    .map(|part_number| {
                        json!({
                            "partNumber": part_number,
                            "url": format!("http://{oss_address}/put/{upload_id}/{part_number}?Signature=local-secret-{part_number}"),
                            "headers": {
                                "Content-Type": "application/octet-stream"
                            }
                        })
                    })
                    .collect::<Vec<_>>();
                json!({
                    "isSuccess": true,
                    "uploadId": upload_id,
                    "expiresAt": 1_800_000_000_000_u64,
                    "parts": signed
                })
            }
            "/java/oss/multipart/complete" => {
                assert_common_body(&body, Some(upload_id));
                let parts = remote_parts.lock().expect("remote parts lock").clone();
                assert_eq!(
                    parts.len(),
                    PART_COUNT as usize,
                    "complete requires all parts"
                );
                for part_number in 1..=PART_COUNT {
                    let part = parts.get(&part_number).expect("complete part");
                    assert_eq!(part.size, expected_part_size(part_number));
                    assert_eq!(part.etag, format!("etag-{part_number}-{}", part.size));
                }
                audit.lock().expect("Java audit lock").complete_seen = true;
                json!({
                    "isSuccess": true,
                    "status": "completed",
                    "uploadId": upload_id,
                    "fileId": file_id,
                    "bucket": "attachment",
                    "objectKey": object_key,
                    "uri": format!("media/{file_id}"),
                    "size": FILE_SIZE,
                    "sha256": FILE_SHA256
                })
            }
            "/java/oss/multipart/abort" => {
                assert_common_body(&body, Some(upload_id));
                audit.lock().expect("Java audit lock").abort_seen = true;
                json!({"isSuccess": true, "status": "aborted", "uploadId": upload_id})
            }
            other => panic!("unexpected Java request path: {other}"),
        };
        audit
            .lock()
            .expect("Java audit lock")
            .requests
            .push(JavaRequestRecord {
                path: path.clone(),
                body,
            });
        write_json_response(&mut stream, &response).await;
        if path == "/java/oss/multipart/complete" {
            break;
        }
    }
}

async fn run_oss_server(
    listener: TcpListener,
    remote_parts: Arc<Mutex<BTreeMap<u32, RemotePart>>>,
    records: Arc<Mutex<Vec<OssPutRecord>>>,
) {
    for _ in 0..PART_COUNT {
        let (mut stream, _) = listener.accept().await.expect("accept OSS PUT");
        let (request_line, headers) = read_headers(&mut stream).await;
        let mut request_parts = request_line.split_whitespace();
        assert_eq!(request_parts.next(), Some("PUT"));
        let target = request_parts.next().expect("OSS PUT target").to_string();
        let path = target.split('?').next().expect("OSS PUT path");
        let mut path_parts = path.split('/');
        assert_eq!(path_parts.next(), Some(""));
        assert_eq!(path_parts.next(), Some("put"));
        assert_eq!(path_parts.next(), Some("upload-multipart-101m"));
        let part_number = path_parts
            .next()
            .expect("OSS part number")
            .parse::<u32>()
            .expect("OSS part number integer");
        let expected_size = expected_part_size(part_number);
        assert_eq!(
            headers
                .get("content-length")
                .and_then(|value| value.parse::<u64>().ok()),
            Some(expected_size)
        );
        assert_eq!(
            headers.get("content-type").map(String::as_str),
            Some("application/octet-stream")
        );
        for forbidden in ["cookieid", "deviceid", "authorization", "x-auth-kind"] {
            assert!(!headers.contains_key(forbidden), "OSS leaked {forbidden}");
        }

        let mut remaining = expected_size;
        let mut chunk = vec![0_u8; 64 * 1024];
        while remaining > 0 {
            let read_len = remaining.min(chunk.len() as u64) as usize;
            stream
                .read_exact(&mut chunk[..read_len])
                .await
                .expect("read OSS range body");
            assert!(
                chunk[..read_len].iter().all(|byte| *byte == 0),
                "OSS range body differs from sparse fixture"
            );
            remaining -= read_len as u64;
        }
        let etag = format!("etag-{part_number}-{expected_size}");
        remote_parts.lock().expect("remote parts lock").insert(
            part_number,
            RemotePart {
                size: expected_size,
                etag: etag.clone(),
            },
        );
        records
            .lock()
            .expect("OSS records lock")
            .push(OssPutRecord {
                part_number,
                size: expected_size,
                etag: etag.clone(),
                target,
                headers,
            });
        let response = format!(
            "HTTP/1.1 200 OK\r\nETag: {etag}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write OSS response");
    }
}

async fn run_go_server(
    listener: TcpListener,
    post_tx: mpsc::UnboundedSender<Value>,
    storage: HostStorage,
) -> Value {
    loop {
        let (mut stream, _) = listener.accept().await.expect("accept Go request");
        let request = read_small_request(&mut stream).await;
        let path = request.target.split('?').next().expect("Go path");
        if path == "/go/channel/load/incrementByChannelId" {
            write_json_response(&mut stream, &json!({"status":"SUCCESS", "data":[]})).await;
            continue;
        }
        if path == "/go/posts/getLatestPost" {
            write_json_response(
                &mut stream,
                &json!({"status":"SUCCESS", "data":{"posts":[],"hasOlder":false,"hasNewer":false}}),
            )
            .await;
            continue;
        }
        assert_eq!(path, "/go/posts/create");
        assert_eq!(request.method, "POST");
        assert_eq!(
            request.headers.get("cookieid").map(String::as_str),
            Some(AUTH_USER_ID)
        );
        assert_eq!(
            request.headers.get("deviceid").map(String::as_str),
            Some(AUTH_DEVICE_ID)
        );
        assert!(!request.headers.contains_key("x-auth-kind"));
        let body: Value = serde_json::from_slice(&request.body).expect("parse Go posts/create");
        for table in ["pending_media", "media_upload_session", "media_upload_part"] {
            assert!(
                storage
                    .scan(ScanSpec {
                        table,
                        limit: None,
                        filter: None,
                        order_by: &[],
                    })
                    .await
                    .expect("scan journal before Go posts/create")
                    .is_empty(),
                "stable PersistOk must clear {table} before Go posts/create"
            );
        }
        post_tx.send(body.clone()).expect("notify Go posts/create");
        write_json_response(
            &mut stream,
            &json!({"status":"SUCCESS", "data":{"id":"psfixx00000000000000000001"}}),
        )
        .await;
        return body;
    }
}

fn ignore_http_outcome(_: &str, _: &Result<HttpResponse, PortError>) -> Option<DomainEventBytes> {
    None
}

async fn apply_im_schema(storage: &HostStorage) {
    for &sql in helix_im::IM_SCHEMA {
        storage
            .execute_raw(sql)
            .await
            .unwrap_or_else(|error| panic!("apply IM_SCHEMA failed for {sql:?}: {error:?}"));
    }
    for &sql in helix_im::IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "apply IM_SCHEMA_MIGRATIONS failed for {sql:?}: {error:?}"
            );
        }
    }
}

fn text_column<'a>(row: &'a Row, name: &str) -> &'a str {
    match row
        .iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
    {
        Some(SqlValue::Text(value)) => value,
        other => panic!("column {name} expected text, got {other:?}"),
    }
}

async fn wait_for_sent_message(storage: &HostStorage) -> Vec<Row> {
    tokio::time::timeout(Duration::from_secs(45), async {
        loop {
            let rows = storage
                .scan(ScanSpec {
                    table: "message",
                    limit: None,
                    filter: None,
                    order_by: &[],
                })
                .await
                .expect("scan message rows");
            if rows.len() == 1 && text_column(&rows[0], "send_status") == "sent" {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("multipart message should reach sent")
}

fn send_command(local_path: &str) -> Tick {
    Tick::Command(helix_core::tick::AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&json!({
            "channel_id": CHANNEL_ID,
            "allocated_temporary_id": TEMPORARY_ID,
            "type": "file",
            "text": "101 MiB multipart integration fixture",
            "props": {
                "file": {"mediaInput": {
                    "localPath": local_path,
                    "fileName": "multipart-101m.bin",
                    "contentType": "application/octet-stream",
                    "size": FILE_SIZE,
                    "sha256": FILE_SHA256
                }}
            }
        }))
        .expect("serialize multipart command"),
    ))
}

fn synthetic_post_echo() -> Tick {
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({
            "v": 1,
            "action": "post",
            "data": {
                "id": "psfixx00000000000000000001",
                "temporaryId": TEMPORARY_ID,
                "channelId": CHANNEL_ID,
                "event_seq": 1,
                "userId": AUTH_USER_ID,
                "type": "FILE",
                "viewers": ["all"],
                "createAt": 1_700_000_000_001_i64
            }
        }))
        .expect("serialize multipart authoritative echo")
        .into(),
    ))
}

fn assert_stable_go_body(body: &Value) {
    let serialized = serde_json::to_string(body).expect("serialize Go body");
    for forbidden in [
        "Signature=",
        "localPath",
        "mediaInput",
        "uploadToken",
        "objectKey",
        "\"headers\"",
        "\"url\"",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "Go body leaked {forbidden}: {serialized}"
        );
    }
    assert_eq!(body["type"], "FILE");
    assert_eq!(body["props"]["file"]["bucket"], "attachment");
    assert_eq!(body["props"]["file"]["uri"], "media/file-multipart-101m");
}

/// LOCAL_INTEGRATION：生产 HostStorage/engine/uploader 链接本地 Java 五接口、OSS range PUT、Go posts/create。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn multipart_pipeline_uses_bounded_ranges_and_stable_reference() {
    let java_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Java listener");
    let java_address = java_listener.local_addr().expect("Java address");
    let oss_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind OSS listener");
    let oss_address = oss_listener.local_addr().expect("OSS address");
    let go_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Go listener");
    let go_address = go_listener.local_addr().expect("Go address");

    let remote_parts = Arc::new(Mutex::new(BTreeMap::<u32, RemotePart>::new()));
    let java_audit = Arc::new(Mutex::new(JavaAudit::default()));
    let oss_records = Arc::new(Mutex::new(Vec::<OssPutRecord>::new()));
    let java_task = tokio::spawn(run_java_server(
        java_listener,
        oss_address,
        Arc::clone(&remote_parts),
        Arc::clone(&java_audit),
    ));
    let oss_task = tokio::spawn(run_oss_server(
        oss_listener,
        Arc::clone(&remote_parts),
        Arc::clone(&oss_records),
    ));
    let (post_tx, mut post_rx) = mpsc::unbounded_channel();

    let fixture_path = std::env::temp_dir().join(format!(
        "helix-multipart-pipeline-{}-{}.bin",
        std::process::id(),
        1_u64
    ));
    let fixture = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&fixture_path)
        .await
        .expect("create sparse fixture");
    fixture
        .set_len(FILE_SIZE)
        .await
        .expect("size sparse fixture to 101 MiB");
    drop(fixture);

    let db = format!(
        "file:helix_multipart_pipeline_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        1_u64
    );
    let host_storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open host SQLite");
    apply_im_schema(&host_storage).await;
    host_storage
        .batch_upsert(UpsertSpec::new(
            "channel",
            vec![vec![
                ("id".to_string(), SqlValue::Text(CHANNEL_ID.to_string())),
                (
                    "team_id".to_string(),
                    SqlValue::Text("tmfixx00000000000000000001".to_string()),
                ),
                (
                    "user_id".to_string(),
                    SqlValue::Text(AUTH_USER_ID.to_string()),
                ),
            ]],
            Some("id"),
        ))
        .await
        .expect("seed channel scope");
    let go_task = tokio::spawn(run_go_server(go_listener, post_tx, host_storage.clone()));

    let java_base = format!("http://{java_address}/java");
    let go_base = format!("http://{go_address}/go");
    let shared_http = SharedHttpClient::new(
        HostNetworkConfig::new(go_base.as_str(), "ws://127.0.0.1:9")
            .with_default_api_base_url(java_base.as_str())
            .with_timeout(Duration::from_secs(30)),
    )
    .expect("build shared HTTP client");
    shared_http
        .headers()
        .set_header_sync("deviceId", AUTH_DEVICE_ID)
        .expect("set platform device identity");
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", AUTH_USER_ID);
    let event_sink = Arc::new(RecordingEvents::default());
    let http = CrossCuttingHttp::new(
        shared_http,
        Arc::clone(&event_sink),
        auth,
        ignore_http_outcome,
    );
    let upload_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build local uploader client");
    let uploader = SharedFileUploader::new(upload_client)
        .with_timeout(Duration::from_secs(30))
        .with_part_timeouts(Duration::from_secs(60), Duration::from_secs(30));

    let channel_id = ChannelId::from_str(CHANNEL_ID).expect("valid channel id");
    let mut module = ImModule::new(ImConfig {
        ws_url: "ws://127.0.0.1:9".to_string(),
        api_base_url: go_base,
        default_api_base_url: java_base,
        auth_user_id: AUTH_USER_ID.to_string(),
        company_id: "tmfixx00000000000000000001".to_string(),
        user_name: "multipart-e2e".to_string(),
        org_name: "test-org".to_string(),
        dept_name: "test-dept".to_string(),
        client_platform: Default::default(),
        ping_interval_ms: 60_000,
        send_timeout_ms: 60_000,
        offline_sync_diagnostics: Default::default(),
    });
    module.register_channel(channel_id, 0);
    let mut shell = ExecutionShell::new();
    shell.register(module);

    let (tick_tx, tick_rx) = mpsc::channel(256);
    let tick_tx_for_loop = tick_tx.clone();
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (_transport_tx, transport_rx) =
        mpsc::unbounded_channel::<TransportRegistration<UnusedFrameSender>>();
    let deps = EngineDeps {
        storage: Arc::new(host_storage.clone()),
        http: Arc::new(http),
        uploader: Arc::new(uploader),
        event_sink: Arc::clone(&event_sink),
        clock: FixedClock,
        trace: TraceHooks::noop(),
        metrics: Arc::new(NoopMetricSink),
        max_http_inflight: 8,
        transport_lifecycle_tx: None,
        transport_trace_tx: None,
    };
    let engine = tokio::spawn(async move {
        run_engine_loop(
            shell,
            tick_rx,
            tick_tx_for_loop,
            deps,
            shutdown_rx,
            HashMap::new(),
            transport_rx,
        )
        .await;
    });

    tick_tx
        .send(Tick::Command(helix_core::tick::AppCommand::new(
            "im_query_messages_by_channel",
            serde_json::to_vec(&json!({
                "channel_id": CHANNEL_ID,
                "pageSize": 20,
                "windowToken": "multipart-local-window"
            }))
            .expect("serialize timeline query"),
        )))
        .await
        .expect("send timeline query");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if event_sink
                .0
                .lock()
                .expect("timeline events lock")
                .iter()
                .any(|event| event["event"] == "im:timeline:ready")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("timeline must be ready before send");

    tick_tx
        .send(send_command(
            fixture_path.to_str().expect("fixture path utf8"),
        ))
        .await
        .expect("send multipart command");
    let post_body = tokio::time::timeout(Duration::from_secs(60), post_rx.recv())
        .await
        .expect("posts/create must follow complete PersistOk")
        .expect("posts/create notification");
    assert_stable_go_body(&post_body);
    tick_tx
        .send(synthetic_post_echo())
        .await
        .expect("send authoritative post echo");

    let final_rows = wait_for_sent_message(&host_storage).await;
    assert_eq!(text_column(&final_rows[0], "temporary_id"), TEMPORARY_ID);
    shutdown_tx.send(()).ok();
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .expect("engine shutdown")
        .expect("engine task");

    let _java_task = tokio::time::timeout(Duration::from_secs(10), java_task)
        .await
        .expect("Java server completion")
        .expect("Java server task");
    let _oss_task = tokio::time::timeout(Duration::from_secs(10), oss_task)
        .await
        .expect("OSS server completion")
        .expect("OSS server task");
    let go_task_body = tokio::time::timeout(Duration::from_secs(10), go_task)
        .await
        .expect("Go server completion")
        .expect("Go server task");
    assert_eq!(go_task_body, post_body);

    let audit = java_audit.lock().expect("Java audit lock");
    assert!(audit.complete_seen);
    assert!(!audit.abort_seen, "successful upload must not abort");
    assert_eq!(
        audit
            .requests
            .iter()
            .filter(|request| request.path == "/java/oss/multipart/init")
            .count(),
        1
    );
    assert_eq!(
        audit
            .requests
            .iter()
            .filter(|request| request.path == "/java/oss/multipart/complete")
            .count(),
        1
    );
    let sign_requests = audit
        .requests
        .iter()
        .filter(|request| request.path == "/java/oss/multipart/sign")
        .collect::<Vec<_>>();
    assert!(
        sign_requests.len() >= 2,
        "101 MiB requires multiple sign batches"
    );
    for request in sign_requests {
        let part_numbers = request.body["partNumbers"]
            .as_array()
            .expect("recorded sign partNumbers");
        assert!((1..=8).contains(&part_numbers.len()));
    }
    let final_parts = audit
        .parts_snapshots
        .iter()
        .rev()
        .find(|snapshot| snapshot.parts.len() == PART_COUNT as usize)
        .expect("Java parts reconciliation must observe all remote ETags");
    assert_eq!(final_parts.status, "completed");
    assert_eq!(final_parts.parts.len(), PART_COUNT as usize);
    drop(audit);

    let records = oss_records.lock().expect("OSS records lock").clone();
    assert_eq!(records.len(), PART_COUNT as usize);
    let mut seen = BTreeSet::new();
    for record in &records {
        assert!(seen.insert(record.part_number));
        assert_eq!(record.size, expected_part_size(record.part_number));
        assert_eq!(
            record.etag,
            format!("etag-{}-{}", record.part_number, record.size)
        );
        assert!(record.target.contains("Signature=local-secret-"));
        assert!(!record.headers.contains_key("cookieid"));
        assert!(!record.headers.contains_key("deviceid"));
    }
    assert_eq!(seen, (1..=PART_COUNT).collect::<BTreeSet<_>>());
    assert_eq!(
        remote_parts.lock().expect("remote parts lock").len(),
        PART_COUNT as usize
    );

    for table in ["pending_media", "media_upload_session", "media_upload_part"] {
        assert!(
            host_storage
                .scan(ScanSpec {
                    table,
                    limit: None,
                    filter: None,
                    order_by: &[],
                })
                .await
                .expect("scan multipart journal")
                .is_empty(),
            "successful multipart run must clear {table}"
        );
    }

    tokio::fs::remove_file(fixture_path)
        .await
        .expect("remove sparse fixture");
}
