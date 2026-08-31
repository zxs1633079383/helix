use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, DomainEventBytes, FileUploadProgress, FileUploadRequest,
    FileUploadResponse, GetSpec, GuardedBumpSpec, HttpResponse, MonotonicUpsertSpec, Row, ScanSpec,
    SqlValue, StorageOp, UpsertSpec,
};
use helix_core::ports::{
    Clock, EventSink, FileUploadProgressReporter, FileUploader, FrameSender, Storage,
};
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
const RICH_TEMPORARY_ID: &str = "tmp-media-pipeline-rich";
const FILE_TEMPORARY_ID: &str = "tmp-media-pipeline-file";
const AUTH_USER_ID: &str = "usfixx00000000000000000001";
const AUTH_DEVICE_ID: &str = "device-media-pipeline";
const IMAGE_BODY: &[u8] = b"rich-image-body";
const IMAGE_SHA256: &str = "f00b8dba93aa1833104e65662acf6a5da7c5d52431e4d70646d6a1c1dde6f291";
const FILE_SIZE: usize = 20 * 64 * 1024;
const FILE_SHA256: &str = "4f934f8b92b4e461b9ad1a7989c2317fd245b275394c4363d891d8998fcf16c6";

static TEST_SEQ: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct LocalHttpRequest {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct JavaRequestRecord {
    path: String,
    body: Value,
}

#[derive(Debug)]
struct OssPutRecord {
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PersistedProgress {
    temporary_id: String,
    percent: i64,
}

#[derive(Clone)]
struct RecordingStorage {
    inner: HostStorage,
    progress: Arc<Mutex<Vec<PersistedProgress>>>,
}

impl RecordingStorage {
    fn new(inner: HostStorage) -> Self {
        Self {
            inner,
            progress: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn progress_snapshot(&self) -> Vec<PersistedProgress> {
        self.progress
            .lock()
            .expect("progress recorder lock")
            .clone()
    }

    fn commit_records(&self, records: Vec<PersistedProgress>) {
        self.progress
            .lock()
            .expect("progress recorder lock")
            .extend(records);
    }
}

#[async_trait::async_trait]
impl Storage for RecordingStorage {
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), PortError> {
        let records = progress_from_upsert(&spec);
        let result = self.inner.batch_upsert(spec).await;
        if result.is_ok() {
            self.commit_records(records);
        }
        result
    }

    async fn batch_update(&self, spec: BatchUpdateSpec) -> Result<(), PortError> {
        let records = progress_from_update(&spec);
        let result = self.inner.batch_update(spec).await;
        if result.is_ok() {
            self.commit_records(records);
        }
        result
    }

    async fn monotonic_upsert(&self, spec: MonotonicUpsertSpec) -> Result<(), PortError> {
        self.inner.monotonic_upsert(spec).await
    }

    async fn guarded_bump(&self, spec: GuardedBumpSpec) -> Result<(), PortError> {
        self.inner.guarded_bump(spec).await
    }

    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, PortError> {
        self.inner.get(spec).await
    }

    async fn scan(&self, spec: ScanSpec) -> Result<Vec<Row>, PortError> {
        self.inner.scan(spec).await
    }

    async fn batch_delete(&self, spec: BatchDeleteSpec) -> Result<(), PortError> {
        self.inner.batch_delete(spec).await
    }

    async fn atomic_write(&self, ops: Vec<StorageOp>) -> Result<(), PortError> {
        let records = ops.iter().flat_map(progress_from_op).collect::<Vec<_>>();
        let result = self.inner.atomic_write(ops).await;
        if result.is_ok() {
            self.commit_records(records);
        }
        result
    }
}

fn progress_from_upsert(spec: &UpsertSpec) -> Vec<PersistedProgress> {
    if spec.table != "message" {
        return Vec::new();
    }
    spec.rows
        .iter()
        .filter_map(|row| {
            let temporary_id = row.iter().find_map(|(column, value)| {
                (column == "temporary_id")
                    .then_some(value)
                    .and_then(|value| match value {
                        SqlValue::Text(value) => Some(value.clone()),
                        _ => None,
                    })
            })?;
            let percent = row.iter().find_map(|(column, value)| {
                (column == "upload_progress_percent")
                    .then_some(value)
                    .and_then(|value| match value {
                        SqlValue::Integer(value) => Some(*value),
                        _ => None,
                    })
            })?;
            Some(PersistedProgress {
                temporary_id,
                percent,
            })
        })
        .collect()
}

fn progress_from_update(spec: &BatchUpdateSpec) -> Vec<PersistedProgress> {
    if spec.table != "message" {
        return Vec::new();
    }
    let Some(percent) = spec.patch.iter().find_map(|(column, value)| {
        (column == "upload_progress_percent")
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Integer(value) => Some(*value),
                _ => None,
            })
    }) else {
        return Vec::new();
    };
    spec.key_vals
        .iter()
        .filter_map(|value| match value {
            SqlValue::Text(temporary_id) => Some(PersistedProgress {
                temporary_id: temporary_id.clone(),
                percent,
            }),
            _ => None,
        })
        .collect()
}

fn progress_from_op(op: &StorageOp) -> Vec<PersistedProgress> {
    match op {
        StorageOp::BatchUpsert(spec) => progress_from_upsert(spec),
        StorageOp::BatchUpdate(spec) => progress_from_update(spec),
        _ => Vec::new(),
    }
}

#[derive(Clone)]
struct RecordingUploader {
    inner: SharedFileUploader,
    progress: Arc<Mutex<Vec<(String, FileUploadProgress)>>>,
}

impl RecordingUploader {
    fn new(inner: SharedFileUploader) -> Self {
        Self {
            inner,
            progress: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn progress_snapshot(&self) -> Vec<(String, FileUploadProgress)> {
        self.progress
            .lock()
            .expect("upload progress recorder lock")
            .clone()
    }
}

struct TeeProgressReporter {
    object_key: String,
    observed: Arc<Mutex<Vec<(String, FileUploadProgress)>>>,
    downstream: Arc<dyn FileUploadProgressReporter>,
}

impl FileUploadProgressReporter for TeeProgressReporter {
    fn report(&self, progress: FileUploadProgress) {
        self.observed
            .lock()
            .expect("upload progress recorder lock")
            .push((self.object_key.clone(), progress));
        self.downstream.report(progress);
    }
}

#[async_trait::async_trait]
impl FileUploader for RecordingUploader {
    async fn upload(&self, req: FileUploadRequest) -> Result<FileUploadResponse, PortError> {
        self.upload_with_progress(req, None).await
    }

    async fn upload_with_progress(
        &self,
        req: FileUploadRequest,
        progress: Option<Arc<dyn FileUploadProgressReporter>>,
    ) -> Result<FileUploadResponse, PortError> {
        let progress = progress.map(|downstream| {
            Arc::new(TeeProgressReporter {
                object_key: req.object_key.clone(),
                observed: Arc::clone(&self.progress),
                downstream,
            }) as Arc<dyn FileUploadProgressReporter>
        });
        self.inner.upload_with_progress(req, progress).await
    }
}

struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _: DomainEventBytes) {}
}

impl BatchSink for NoopSink {}

#[derive(Clone)]
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

async fn read_http_request(stream: &mut TcpStream) -> LocalHttpRequest {
    let mut request = Vec::new();
    let expected_total = loop {
        if let Some(header_end) = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| index + 4)
        {
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    if name.eq_ignore_ascii_case("content-length") {
                        value.trim().parse::<usize>().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            let expected_total = header_end + content_length;
            if request.len() >= expected_total {
                break expected_total;
            }
        }

        let mut chunk = [0_u8; 16 * 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .expect("read local HTTP request");
        assert!(read > 0, "local HTTP request ended before its body");
        request.extend_from_slice(&chunk[..read]);
    };

    let header_end = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .expect("local HTTP request headers");
    let headers_text = String::from_utf8_lossy(&request[..header_end]);
    let mut lines = headers_text.lines();
    let request_line = lines.next().expect("local HTTP request line");
    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts
        .next()
        .expect("local HTTP request method")
        .to_string();
    let target = request_line_parts
        .next()
        .expect("local HTTP request target")
        .to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    LocalHttpRequest {
        method,
        target,
        headers,
        body: request[header_end..expected_total].to_vec(),
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
        .expect("write local HTTP response headers");
    stream
        .write_all(&body)
        .await
        .expect("write local HTTP response body");
}

async fn run_java_server(
    listener: TcpListener,
    oss_address: std::net::SocketAddr,
) -> Vec<JavaRequestRecord> {
    let mut records = Vec::with_capacity(10);
    let mut prepare_count = 0usize;
    for _ in 0..10 {
        let (mut stream, _) = listener.accept().await.expect("accept Java request");
        let request = read_http_request(&mut stream).await;
        assert_eq!(request.method, "POST");
        let path = request
            .target
            .split('?')
            .next()
            .expect("Java request path")
            .to_string();
        let body: Value =
            serde_json::from_slice(&request.body).expect("parse Java request JSON body");
        assert_eq!(body["mediaUpload"].as_bool(), Some(true));
        assert_eq!(
            request.headers.get("cookieid").map(String::as_str),
            Some(AUTH_USER_ID),
            "Java media control plane must receive the platform-injected session credential"
        );
        assert_eq!(
            request.headers.get("deviceid").map(String::as_str),
            Some(AUTH_DEVICE_ID),
            "Java media control plane must receive the platform-injected device identity"
        );
        assert!(
            !request.headers.contains_key("x-auth-kind"),
            "internal auth intent must be stripped before Java sees the request"
        );

        let response = match path.as_str() {
            "/java/oss/getPresignedUrl" => {
                prepare_count += 1;
                let file_id = format!("file-{prepare_count:02}");
                let object_key = format!("media/{file_id}");
                let content_type = body["contentType"].as_str().expect("prepare contentType");
                let expected_bucket = if content_type.starts_with("image/") {
                    "picture"
                } else {
                    "attachment"
                };
                assert_eq!(body["bucket"].as_str(), Some(expected_bucket));
                assert!(body["clientUploadId"].as_str().is_some());
                assert!(body["size"].as_u64().is_some());
                assert!(body["sha256"].as_str().is_some());
                json!({
                    "isSuccess": true,
                    "fileId": file_id,
                    "bucket": expected_bucket,
                    "method": "PUT",
                    "url": format!(
                        "http://{oss_address}/put/{object_key}?Signature=write-secret-{prepare_count:02}"
                    ),
                    "uri": object_key.clone(),
                    "objectKey": object_key,
                    "headers": {
                        "Content-Type": content_type,
                        "x-oss-forbid-overwrite": "true"
                    }
                })
            }
            other => panic!("unexpected Java request path: {other}"),
        };
        records.push(JavaRequestRecord { path, body });
        write_json_response(&mut stream, &response).await;
    }
    assert_eq!(prepare_count, 10);
    records
}

async fn run_oss_server(
    listener: TcpListener,
    uploaded: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
) -> Vec<OssPutRecord> {
    let mut records = Vec::with_capacity(10);
    for ordinal in 1..=10 {
        let (mut stream, _) = listener.accept().await.expect("accept OSS PUT");
        let request = read_http_request(&mut stream).await;
        assert_eq!(request.method, "PUT");
        assert_eq!(
            request
                .headers
                .get("x-oss-forbid-overwrite")
                .map(String::as_str),
            Some("true")
        );
        for forbidden in ["cookieid", "deviceid", "authorization", "x-auth-kind"] {
            assert!(
                !request.headers.contains_key(forbidden),
                "OSS PUT must not receive platform credential header {forbidden}"
            );
        }
        let path = request.target.split('?').next().expect("OSS PUT path");
        let object_key = path
            .strip_prefix("/put/")
            .expect("OSS PUT object key")
            .to_string();
        uploaded
            .lock()
            .expect("uploaded object map lock")
            .insert(object_key, request.body.clone());

        let response = format!(
            "HTTP/1.1 200 OK\r\nETag: etag-put-{ordinal:02}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write OSS PUT response");
        records.push(OssPutRecord {
            target: request.target,
            headers: request.headers,
            body: request.body,
        });
    }
    records
}

async fn run_go_server(listener: TcpListener, post_tx: mpsc::UnboundedSender<usize>) -> Vec<Value> {
    let mut posts = Vec::with_capacity(2);
    for ordinal in 1..=2 {
        let (mut stream, _) = listener.accept().await.expect("accept Go request");
        let request = read_http_request(&mut stream).await;
        assert_eq!(request.method, "POST");
        assert_eq!(request.target, "/go/posts/create");
        assert_eq!(
            request.headers.get("cookieid").map(String::as_str),
            Some(AUTH_USER_ID),
            "Go posts/create must receive the same platform session credential"
        );
        assert_eq!(
            request.headers.get("deviceid").map(String::as_str),
            Some(AUTH_DEVICE_ID),
            "Go posts/create must receive the same platform device identity"
        );
        assert!(
            !request.headers.contains_key("x-auth-kind"),
            "internal auth intent must be stripped before Go sees the request"
        );
        let body: Value =
            serde_json::from_slice(&request.body).expect("parse Go posts/create body");
        posts.push(body);
        post_tx
            .send(ordinal)
            .expect("notify test about Go posts/create");
        write_json_response(
            &mut stream,
            &json!({
                "status": "SUCCESS",
                "data": {"id": format!("psfixx{ordinal:020x}")}
            }),
        )
        .await;
    }
    posts
}

fn is_duplicate_column_error(error: &impl std::fmt::Display) -> bool {
    error.to_string().contains("duplicate column name")
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
                is_duplicate_column_error(&error),
                "apply IM_SCHEMA_MIGRATIONS failed for {sql:?}: {error:?}"
            );
        }
    }
}

fn command(payload: Value) -> Tick {
    Tick::Command(helix_core::tick::AppCommand::new(
        "im_send_message",
        serde_json::to_vec(&payload) // hot-path-audit: ignore -- test command boundary, not a DB write
            .expect("serialize im_send_message payload"),
    ))
}

fn rich_command(local_path: &str) -> Tick {
    let files = (0..9)
        .map(|index| {
            json!({
                "width": 800 + index,
                "height": 600 + index,
                "mediaInput": {
                    "localPath": local_path,
                    "fileName": format!("image-{index}.png"),
                    "contentType": "image/png",
                    "size": IMAGE_BODY.len(),
                    "sha256": IMAGE_SHA256
                }
            })
        })
        .collect::<Vec<_>>();
    command(json!({
        "channel_id": CHANNEL_ID,
        "allocated_temporary_id": RICH_TEMPORARY_ID,
        "type": "rich",
        "text": "nine images",
        "props": {
            "imagePosition": "bottom",
            "files": files
        }
    }))
}

fn file_command(local_path: &str) -> Tick {
    command(json!({
        "channel_id": CHANNEL_ID,
        "allocated_temporary_id": FILE_TEMPORARY_ID,
        "type": "file",
        "text": "large report",
        "props": {
            "file": {
                "mediaInput": {
                    "localPath": local_path,
                    "fileName": "report.bin",
                    "contentType": "application/octet-stream",
                    "size": FILE_SIZE,
                    "sha256": FILE_SHA256
                }
            }
        }
    }))
}

fn column<'a>(row: &'a Row, name: &str) -> Option<&'a SqlValue> {
    row.iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
}

fn text_column<'a>(row: &'a Row, name: &str) -> &'a str {
    match column(row, name) {
        Some(SqlValue::Text(value)) => value,
        other => panic!("column {name} expected text, got {other:?}"),
    }
}

fn integer_column(row: &Row, name: &str) -> i64 {
    match column(row, name) {
        Some(SqlValue::Integer(value)) => *value,
        other => panic!("column {name} expected integer, got {other:?}"),
    }
}

async fn wait_for_sent_messages(storage: &HostStorage) -> Vec<Row> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let rows = storage
                .scan(ScanSpec {
                    table: "message",
                    limit: None,
                    filter: None,
                    order_by: &[],
                })
                .await
                .expect("scan final message rows");
            if rows.len() == 2
                && rows
                    .iter()
                    .all(|row| text_column(row, "send_status") == "sent")
            {
                return rows;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("both media messages should reach sent in real SQLite")
}

fn assert_media_go_body(body: &Value) {
    let serialized = serde_json::to_string(body).expect("serialize Go body for leak assertion");
    for forbidden in [
        "Signature=",
        "write-secret",
        "localPath",
        "mediaInput",
        "uploadToken",
        "objectKey",
        "\"headers\"",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "Go body leaked transient upload field {forbidden}: {serialized}"
        );
    }

    let references = match body["type"].as_str() {
        Some("RICH") => body["props"]["files"]
            .as_array()
            .expect("rich props.files")
            .iter()
            .map(|file| {
                (
                    file["bucket"].as_str().expect("rich bucket"),
                    file["uri"].as_str().expect("rich uri"),
                )
            })
            .collect::<Vec<_>>(),
        Some("FILE") => vec![(
            body["props"]["file"]["bucket"]
                .as_str()
                .expect("file bucket"),
            body["props"]["file"]["uri"].as_str().expect("file uri"),
        )],
        other => panic!("unexpected Go media type: {other:?}"),
    };
    let expected_bucket = if body["type"] == "RICH" {
        "picture"
    } else {
        "attachment"
    };
    assert!(references
        .iter()
        .all(|(bucket, uri)| *bucket == expected_bucket && uri.starts_with("media/file-")));
    assert!(!serialized.contains("\"url\""));
}

fn ignore_http_outcome(_: &str, _: &Result<HttpResponse, PortError>) -> Option<DomainEventBytes> {
    None
}

/// 构造本地夹具模拟的权威 WS echo；真实 Go/WS 证据由独立 Native real-chain target 提供。
fn synthetic_post_echo_tick(
    temporary_id: &str,
    server_id: &str,
    event_seq: u64,
    msg_type: &str,
) -> Tick {
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({
            "v": 1,
            "action": "post",
            "data": {
                "id": server_id,
                "temporaryId": temporary_id,
                "channelId": CHANNEL_ID,
                "event_seq": event_seq,
                "userId": AUTH_USER_ID,
                "type": msg_type,
                "viewers": ["all"],
                "createAt": 1_700_000_000_000_i64 + event_seq as i64
            }
        }))
        .expect("serialize authoritative media echo")
        .into(),
    ))
}

/// 验证 RICH/FILE 通过本地 HTTP/echo 夹具共用管线并最终写入真实 SQLite。
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn rich_and_file_share_one_host_pipeline_across_java_oss_and_go_origins() {
    let java_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local Java server");
    let java_address = java_listener.local_addr().expect("local Java address");
    let oss_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local OSS server");
    let oss_address = oss_listener.local_addr().expect("local OSS address");
    let go_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local Go server");
    let go_address = go_listener.local_addr().expect("local Go address");
    let java_base = format!("http://{java_address}/java");
    let go_base = format!("http://{go_address}/go");

    let uploaded = Arc::new(Mutex::new(BTreeMap::new()));
    let java_server = tokio::spawn(run_java_server(java_listener, oss_address));
    let oss_server = tokio::spawn(run_oss_server(oss_listener, Arc::clone(&uploaded)));
    let (go_post_tx, mut go_post_rx) = mpsc::unbounded_channel();
    let go_server = tokio::spawn(run_go_server(go_listener, go_post_tx));

    let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
    let image_path = std::env::temp_dir().join(format!(
        "helix-media-pipeline-{}-{seq}.png",
        std::process::id()
    ));
    let file_path = std::env::temp_dir().join(format!(
        "helix-media-pipeline-{}-{seq}.bin",
        std::process::id()
    ));
    tokio::fs::write(&image_path, IMAGE_BODY)
        .await
        .expect("write rich image fixture");
    tokio::fs::write(&file_path, vec![b'f'; FILE_SIZE])
        .await
        .expect("write file fixture");

    let db = format!(
        "file:helix_media_pipeline_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let host_storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open media pipeline SQLite");
    apply_im_schema(&host_storage).await;
    let storage = RecordingStorage::new(host_storage.clone()); // hot-path-audit: ignore -- shared test handle

    let shared_http = SharedHttpClient::new(
        HostNetworkConfig::new(go_base.as_str(), "ws://127.0.0.1:9")
            .with_default_api_base_url(java_base.as_str())
            .with_timeout(Duration::from_secs(10)),
    )
    .expect("build shared HTTP client with independent Go and Java bases");
    shared_http
        .headers()
        .set_header_sync("deviceId", AUTH_DEVICE_ID)
        .expect("inject platform device identity");
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", AUTH_USER_ID);
    let event_sink = Arc::new(NoopSink);
    let http = CrossCuttingHttp::new(
        shared_http,
        Arc::clone(&event_sink),
        auth,
        ignore_http_outcome,
    );
    let upload_client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("build isolated upload client");
    let uploader = RecordingUploader::new(
        SharedFileUploader::new(upload_client).with_timeout(Duration::from_secs(10)),
    );

    let channel_id = ChannelId::from_str(CHANNEL_ID).expect("valid media pipeline channel");
    let mut module = ImModule::new(ImConfig {
        ws_url: "ws://127.0.0.1:9".to_string(),
        api_base_url: go_base.clone(), // hot-path-audit: ignore -- one-shot test assembly
        default_api_base_url: java_base.clone(), // hot-path-audit: ignore -- one-shot test assembly
        auth_user_id: AUTH_USER_ID.to_string(),
        company_id: "tmfixx00000000000000000001".to_string(),
        user_name: "media-e2e".to_string(),
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
    let tick_tx_for_loop = tick_tx.clone(); // hot-path-audit: ignore -- channel handle
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (_transport_tx, transport_rx) =
        mpsc::unbounded_channel::<TransportRegistration<UnusedFrameSender>>();
    let deps = EngineDeps {
        storage: Arc::new(storage.clone()), // hot-path-audit: ignore -- recorder handle
        http: Arc::new(http),
        uploader: Arc::new(uploader.clone()), // hot-path-audit: ignore -- recorder handle
        event_sink,
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
        .send(rich_command(
            image_path.to_str().expect("UTF-8 rich image fixture path"),
        ))
        .await
        .expect("send rich command");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(15), go_post_rx.recv())
            .await
            .expect("rich posts/create should arrive")
            .expect("Go post notification channel"),
        1
    );
    tick_tx
        .send(synthetic_post_echo_tick(
            RICH_TEMPORARY_ID,
            "psfixx00000000000000000001",
            1,
            "RICH",
        ))
        .await
        .expect("send rich authoritative echo");
    assert!(
        uploader.progress_snapshot().is_empty(),
        "rich image uploads must not install a progress reporter"
    );
    assert!(
        storage
            .progress_snapshot()
            .iter()
            .all(|progress| progress.temporary_id != RICH_TEMPORARY_ID || progress.percent == 0),
        "rich image completion must not create percentage progress writes"
    );

    tick_tx
        .send(file_command(
            file_path.to_str().expect("UTF-8 file fixture path"),
        ))
        .await
        .expect("send file command");
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(15), go_post_rx.recv())
            .await
            .expect("file posts/create should arrive")
            .expect("Go post notification channel"),
        2
    );
    tick_tx
        .send(synthetic_post_echo_tick(
            FILE_TEMPORARY_ID,
            "psfixx00000000000000000002",
            2,
            "FILE",
        ))
        .await
        .expect("send file authoritative echo");

    let final_rows = wait_for_sent_messages(&host_storage).await;
    shutdown_tx.send(()).ok();
    tokio::time::timeout(Duration::from_secs(5), engine)
        .await
        .expect("engine should stop after shutdown")
        .expect("engine task should not panic");

    let java_records = tokio::time::timeout(Duration::from_secs(5), java_server)
        .await
        .expect("Java server should receive all requests")
        .expect("Java server task should not panic");
    let oss_records = tokio::time::timeout(Duration::from_secs(5), oss_server)
        .await
        .expect("OSS server should receive all PUTs")
        .expect("OSS server task should not panic");
    let go_posts = tokio::time::timeout(Duration::from_secs(5), go_server)
        .await
        .expect("Go server should receive both posts")
        .expect("Go server task should not panic");

    let prepare_records = java_records
        .iter()
        .filter(|record| record.path == "/java/oss/getPresignedUrl")
        .collect::<Vec<_>>();
    assert_eq!(prepare_records.len(), 10);
    assert_eq!(java_records.len(), 10);
    assert_eq!(
        prepare_records
            .iter()
            .filter_map(|record| record.body["clientUploadId"].as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        10,
        "every attachment must receive an independent Java upload ticket"
    );

    assert_eq!(oss_records.len(), 10);
    assert_eq!(
        oss_records
            .iter()
            .filter(|record| record.body.as_slice() == IMAGE_BODY)
            .count(),
        9
    );
    assert_eq!(
        oss_records
            .iter()
            .filter(|record| {
                record.body.len() == FILE_SIZE && record.body.iter().all(|byte| *byte == b'f')
            })
            .count(),
        1
    );
    assert!(oss_records.iter().all(|record| {
        record.target.contains("Signature=write-secret-")
            && record
                .headers
                .get("x-oss-forbid-overwrite")
                .map(String::as_str)
                == Some("true")
    }));

    assert_eq!(go_posts.len(), 2);
    assert_eq!(go_posts[0]["type"], "RICH");
    assert_eq!(
        go_posts[0]["props"]["files"]
            .as_array()
            .expect("rich Go props.files")
            .len(),
        9
    );
    assert_eq!(go_posts[1]["type"], "FILE");
    assert_media_go_body(&go_posts[0]);
    assert_media_go_body(&go_posts[1]);

    let raw_file_progress = uploader.progress_snapshot();
    assert!(
        raw_file_progress
            .first()
            .is_some_and(|(_, progress)| progress.completed_bytes < progress.total_bytes),
        "file upload must expose at least one intermediate byte progress sample"
    );
    assert!(
        raw_file_progress
            .last()
            .is_some_and(|(_, progress)| progress.completed_bytes == progress.total_bytes),
        "file upload must expose its terminal byte progress sample"
    );
    assert!(
        raw_file_progress
            .iter()
            .all(|(object_key, _)| object_key.starts_with("media/file-")),
        "only the FILE upload may install the progress reporter"
    );

    let file_progress = storage
        .progress_snapshot()
        .into_iter()
        .filter(|progress| progress.temporary_id == FILE_TEMPORARY_ID && progress.percent > 0)
        .map(|progress| progress.percent)
        .collect::<Vec<_>>();
    assert!(
        file_progress
            .iter()
            .any(|percent| (5..100).contains(percent)),
        "FILE must persist at least one intermediate 5% progress bucket: {file_progress:?}"
    );
    assert_eq!(
        file_progress.first(),
        Some(&5),
        "a 20 × 64 KiB fixture must make the first persisted progress bucket exactly 5%"
    );
    assert_eq!(file_progress.last(), Some(&100));
    assert!(
        file_progress.windows(2).all(|pair| pair[0] <= pair[1])
            && file_progress
                .iter()
                .all(|percent| *percent > 0 && *percent <= 100 && percent % 5 == 0),
        "FILE progress must be monotone 5% buckets: {file_progress:?}"
    );

    let rich_row = final_rows
        .iter()
        .find(|row| text_column(row, "temporary_id") == RICH_TEMPORARY_ID)
        .expect("final rich SQLite row");
    let file_row = final_rows
        .iter()
        .find(|row| text_column(row, "temporary_id") == FILE_TEMPORARY_ID)
        .expect("final file SQLite row");
    assert_eq!(text_column(rich_row, "send_status"), "sent");
    assert_eq!(text_column(file_row, "send_status"), "sent");
    assert_eq!(integer_column(rich_row, "upload_progress_percent"), 0);
    assert_eq!(integer_column(file_row, "upload_progress_percent"), 100);
    assert!(
        host_storage
            .scan(ScanSpec {
                table: "pending_media",
                limit: None,
                filter: None,
                order_by: &[],
            })
            .await
            .expect("scan pending_media after success")
            .is_empty(),
        "successful media pipeline must clear its durable recovery journal"
    );

    tokio::fs::remove_file(image_path)
        .await
        .expect("remove rich image fixture");
    tokio::fs::remove_file(file_path)
        .await
        .expect("remove file fixture");
}
