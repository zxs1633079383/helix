#![allow(dead_code)]
//! MessageV3 Phase1-5 的 Rust→真实 Go 集成测试。
//!
//! 这不是 fake server、fixture 回放或 Node 编排器：测试通过 native driver 的真实
//! `NativeHttp`、`NativeTransport`、`NativeStorage` 和 `NativeEventSink`，把 Helix
//! `ExecutionShell` 接到当前 profile 的 Go 服务，并在同一轮核对 HTTP、WS、SQLite 与 emit。
//!
//! 运行方式：
//!
//! ```text
//! cargo test -p helix-driver-native --test message_v3_domain -- --nocapture --test-threads=1
//! ```
//!
//! 默认使用 `pre` profile 的非秘密身份；可用 `MESSAGEV3_ENV` 与端点变量覆盖运行环境。
//! Phase1-5 共用同一套 Go/WS 前置。Go、登录服务或对象存储不可达时显式失败为
//! `BLOCKED_PRECONDITION/NOT_VERIFIED`，不会把 health 200、HTTP admission 或 fake WS 当成 PASS。

use bytes::Bytes;
use helix_core::effect::{
    DomainEventBytes, Effect, HttpRequest, Row, ScanOrder, ScanSpec, ScopedGetSpec, SqlValue,
    TransportId,
};
use helix_core::module_host::Module;
use helix_core::ports::{EventSink, HttpRequester, Storage};
use helix_core::tick::{AppCommand, Tick};
use helix_core::ExecutionShell;
use helix_driver_host::{execute_storage_ops, AsyncMetricSink, HostNetworkConfig, NoopMetricSink};
use helix_driver_native::{
    bus_event_name, run_engine_loop_with_transports, to_bus_envelope, EngineConfig,
    NativeEventSink, NativeHttp, NativeStorage, NativeTransport, SharedFileUploader,
    TransportTable,
};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, OnceCell};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

mod chain;
mod metrics;
#[allow(unused_imports)]
pub(crate) use chain::run_text_chain_gate;
pub(crate) use chain::run_unimplemented_chain_gate;

const PRE_GO_HTTP_BASE: &str = "http://api-im-pre.jinqidongli.com/api/cses";
const PRE_GO_WS_URL: &str = "ws://ws-im-pre.jinqidongli.com/api/v4/websocket";
const PRE_GO_HEALTH_URL: &str = "http://api-im-pre.jinqidongli.com/api/cses/health";
const LOCAL_GO_HTTP_BASE: &str = "http://127.0.0.1:18066/api/cses";
const LOCAL_GO_WS_URL: &str = "ws://127.0.0.1:18066/api/v4/websocket";
const LOCAL_GO_HEALTH_URL: &str = "http://127.0.0.1:18066/api/cses/health";
const CHAIN_COMPANY_ID: &str = "64118eebd2b665246b7880eb";
const CHAIN_ACTOR_USER_ID: &str = "444";
const CHAIN_OBSERVER_USER_ID: &str = "678";
const CHAIN_THIRD_USER_ID: &str = "447";
// Search 只使用远端 ES 中实时发现的只读消息；关键词固定，频道/帖子 ID 不固化。
const SEARCH_FIXTURE_KEYWORD: &str = "1";
const PRE_JAVA_API_BASE: &str = "https://api-pre.jinqidongli.com";
const LOCAL_JAVA_API_BASE: &str = "http://127.0.0.1:3399";
pub(crate) const MAIN_API_BASE: &str = PRE_JAVA_API_BASE;
pub(crate) const CREDENTIALS_RELATIVE: &str =
    "angular/visual/cses-client/.wdio/credentials.local.json";

/// 判断当前 real-chain profile 是否显式选择本地运行态。
fn is_local_messagev3_profile() -> bool {
    matches!(
        std::env::var("MESSAGEV3_ENV")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("local")
    )
}

/// 返回可覆盖的 Go HTTP 基地址；未设置变量时按 profile 选择 pre/local 默认值。
pub(crate) fn go_http_base() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            std::env::var("MESSAGEV3_GO_HTTP_BASE")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    if is_local_messagev3_profile() {
                        LOCAL_GO_HTTP_BASE.to_string()
                    } else {
                        PRE_GO_HTTP_BASE.to_string()
                    }
                })
        })
        .as_str()
}

/// 返回可覆盖的 Go WebSocket 地址；未设置变量时按 profile 选择 pre/local 默认值。
pub(crate) fn go_ws_url() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            std::env::var("MESSAGEV3_GO_WS_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    if is_local_messagev3_profile() {
                        LOCAL_GO_WS_URL.to_string()
                    } else {
                        PRE_GO_WS_URL.to_string()
                    }
                })
        })
        .as_str()
}

/// 返回可覆盖的 Go health 地址；未设置变量时按 profile 选择 pre/local 默认值。
pub(crate) fn go_health_url() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            std::env::var("MESSAGEV3_GO_HEALTH_URL")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| {
                    if is_local_messagev3_profile() {
                        LOCAL_GO_HEALTH_URL.to_string()
                    } else {
                        PRE_GO_HEALTH_URL.to_string()
                    }
                })
        })
        .as_str()
}

/// Phase5 与 Phase1-4 共用同一套可配置 Go HTTP 端点。
pub(crate) fn chain_go_http_base() -> &'static str {
    go_http_base()
}

/// Phase5 与 Phase1-4 共用同一套可配置 Go WebSocket 端点。
pub(crate) fn chain_go_ws_url() -> &'static str {
    go_ws_url()
}

/// 返回 driver-native 登录服务地址；默认 profile 为 pre，允许单独覆盖 Java API。
pub(crate) fn main_api_base() -> String {
    std::env::var("MESSAGEV3_JAVA_API_BASE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("HELIX_MAIN_API_BASE")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| {
            if is_local_messagev3_profile() {
                LOCAL_JAVA_API_BASE.to_string()
            } else {
                MAIN_API_BASE.to_string()
            }
        })
}

pub(crate) static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
pub(crate) static AUTH_CONTEXT: OnceCell<Result<Arc<AuthContext>, String>> = OnceCell::const_new();
pub(crate) static CHAIN_AUTH_CONTEXT: OnceCell<Result<Arc<AuthContext>, String>> =
    OnceCell::const_new();
pub(crate) static SEARCH_AUTH_CONTEXT: OnceCell<Result<Arc<AuthContext>, String>> =
    OnceCell::const_new();
pub(crate) static SEARCH_FIXTURE: OnceCell<Result<Arc<SearchFixture>, String>> =
    OnceCell::const_new();
pub(crate) static REMOTE_LOCK: OnceCell<Mutex<()>> = OnceCell::const_new();

const CHANNEL_FIXTURE_WAIT_SECS: u64 = 30;
const CHANNEL_FIXTURE_FRESHNESS_MS: i64 = 10 * 60 * 1000;
const CHANNEL_FIXTURE_SNAPSHOT_END_MS: i64 = 9_000_000_000_000;
const PHASE_POOL_MAX_CHANNELS: usize = 5;
const PHASE_POOL_PING_INTERVAL_DEFAULT_MS: u64 = 15_000;
const PHASE_POOL_EVENT_CAPACITY: usize = 16_384;
const SESSION_SETUP_TIMEOUT_SECS: u64 = 30;
const PHASE_RUNTIME_TASK_TIMEOUT_SECS: u64 = 60;
const SEARCH_PREFLIGHT_TIMEOUT_SECS: u64 = 8;
const CHAIN_ENTRY_ORDER: &[ScanOrder] = &[ScanOrder::asc("seq")];

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum CredentialFile {
    /// Angular 账号池当前使用的数组格式。
    Array(Vec<CredentialAccount>),
    /// 兼容包含 `accounts` 与可选默认索引的对象格式。
    Object {
        accounts: Vec<CredentialAccount>,
        #[serde(default, alias = "defaultIndex")]
        default_index: usize,
    },
}

impl CredentialFile {
    /// 统一读取两种账号池格式的账号切片，避免认证逻辑感知文件形状。
    fn accounts(&self) -> &[CredentialAccount] {
        match self {
            Self::Array(accounts) => accounts,
            Self::Object { accounts, .. } => accounts,
        }
    }

    /// 统一读取对象格式的默认账号索引，数组格式默认从零开始。
    fn default_index(&self) -> usize {
        match self {
            Self::Array(_) => 0,
            Self::Object { default_index, .. } => *default_index,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CredentialAccount {
    pub(crate) mobile: Option<String>,
    pub(crate) password: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Persona {
    pub(crate) user_id: String,
    pub(crate) cookie_id: String,
    pub(crate) company_id: String,
    pub(crate) user_name: String,
    pub(crate) org_name: String,
    pub(crate) dept_name: String,
    pub(crate) companies: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct AuthContext {
    pub(crate) actor: Persona,
    pub(crate) observer: Persona,
    pub(crate) third: Persona,
}

/// Search Gate 的只读运行时夹具；频道与帖子 ID 从当前 ES 结果动态取出，避免过期常量。
#[derive(Clone, Debug)]
pub(crate) struct SearchFixture {
    pub(crate) context: Arc<AuthContext>,
    pub(crate) channel_id: String,
    pub(crate) post_id: String,
    pub(crate) keyword: String,
}

#[derive(Clone, Debug)]
struct SearchFixtureSeed {
    channel_id: String,
    post_id: String,
}

#[derive(Debug)]
pub(crate) struct HttpObservation {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct RawWs {
    pub(crate) transport: Arc<NativeTransport>,
    pub(crate) ticks: mpsc::Receiver<Tick>,
    pending: VecDeque<Value>,
}

const MAX_PENDING_WS_FRAMES: usize = 256;

impl RawWs {
    /// 先释放不再消费的观察队列，避免关闭的Disconnected被满队列反压。
    async fn close(self) -> Result<(), String> {
        let Self {
            transport,
            ticks,
            pending,
        } = self;
        drop(ticks);
        drop(pending);
        close_send_transport(transport, "Go observer WS").await
    }

    /// 从缓存帧中取出当前等待器可消费的第一帧，保留跨 action 的乱序广播。
    fn take_pending<F>(&mut self, actions: &[&str], predicate: &F) -> Option<Value>
    where
        F: Fn(&Value) -> bool,
    {
        let index = self.pending.iter().position(|frame| {
            let action = frame.get("action").and_then(Value::as_str);
            actions.iter().any(|candidate| Some(*candidate) == action) && predicate(frame)
        })?;
        self.pending.remove(index)
    }

    /// 接收并解析一帧真实 WS 入站 JSON，忽略 transport 控制帧和非 JSON 数据。
    async fn receive_frame(&mut self) -> Result<Value, String> {
        loop {
            let tick = self
                .ticks
                .recv()
                .await
                .ok_or_else(|| "real Go WS tick channel closed".to_string())?;
            let Tick::Inbound(bytes) = tick else {
                continue;
            };
            let Ok(frame) = serde_json::from_slice::<Value>(bytes.as_bytes()) else {
                continue;
            };
            return Ok(frame);
        }
    }

    /// 缓存当前等待器不能消费但后续 action 可能需要的业务帧，并限制内存上界。
    fn remember(&mut self, frame: Value) {
        if frame.get("action").and_then(Value::as_str).is_none() {
            return;
        }
        if self.pending.len() >= MAX_PENDING_WS_FRAMES {
            self.pending.pop_front();
        }
        self.pending.push_back(frame);
    }
}

#[derive(Clone)]
pub(crate) struct SendIntent {
    pub(crate) message_type: &'static str,
    pub(crate) text: String,
    pub(crate) props: Value,
    pub(crate) viewers: Vec<String>,
    pub(crate) mentions: Vec<String>,
    pub(crate) replied: Option<Value>,
}

/// 返回 Go wire 层的规范消息类型；旧 IMAGE 入参在发送合同中落为 RICH。
fn canonical_wire_message_type(message_type: &str) -> &str {
    match message_type {
        "IMAGE" => "RICH",
        other => other,
    }
}

/// 构造媒体 Gate 的元数据 fixture；发送前由 `stage_media_inputs` 换成真实 mediaInput 字节。
///
/// 这里保留 Gate 声明的文件名、MIME、尺寸和展示字段，不把固定 id/hash 当成上传证据。
pub(crate) fn completed_media_ref(
    bucket: &str,
    id: &str,
    name: &str,
    content_type: &str,
    size: u64,
    sha256: &str,
    extra: Value,
) -> Value {
    let mut node = serde_json::Map::from_iter([
        ("bucket".to_string(), json!(bucket)),
        ("id".to_string(), json!(id)),
        ("uri".to_string(), json!(format!("/media/{id}"))),
        ("name".to_string(), json!(name)),
        ("contentType".to_string(), json!(content_type)),
        ("size".to_string(), json!(size)),
        ("sha256".to_string(), json!(sha256)),
        ("upload".to_string(), json!({"status": "completed"})),
    ]);
    if let Some(extra) = extra.as_object() {
        node.extend(extra.clone());
    }
    Value::Object(node)
}

#[derive(Clone, Copy, Debug)]
enum MediaSlot {
    Files(usize),
    File,
    TemplateFile,
}

#[derive(Clone, Debug)]
struct StagedMedia {
    slot: MediaSlot,
    name: String,
    content_type: String,
    size: u64,
    sha256: String,
    bucket: &'static str,
    duration: Option<f64>,
    width: Option<u64>,
    height: Option<u64>,
    local_path: PathBuf,
}

/// 为 real-chain 生成小型、可复验的媒体字节；哈希来自实际写入 OSS 的字节而非 fixture 常量。
fn media_fixture_bytes(size: u64, seed: &str) -> Result<(Vec<u8>, String), String> {
    const MAX_MEDIA_SIZE: u64 = 64 << 20;
    if !(1..=MAX_MEDIA_SIZE).contains(&size) {
        return Err(format!(
            "media fixture size must be 1..={MAX_MEDIA_SIZE}, got {size}"
        ));
    }
    let size =
        usize::try_from(size).map_err(|_| "media fixture size exceeds host usize".to_string())?;
    let seed_digest = Sha256::digest(seed.as_bytes());
    let mut bytes = Vec::with_capacity(size);
    for index in 0..size {
        let salt = (index as u64).wrapping_mul(31) as u8;
        bytes.push(seed_digest[index % seed_digest.len()] ^ salt);
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok((bytes, sha256))
}

/// 把媒体名称收敛成安全临时文件名，不把用户路径或签名 URL 带入文件系统。
fn media_fixture_path(gate: &str, ordinal: usize, name: &str) -> PathBuf {
    let safe_gate = gate
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let safe_name = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    std::env::temp_dir().join(format!(
        "helix-messagev3-{safe_gate}-{}-{ordinal}-{safe_name}",
        std::process::id()
    ))
}

/// 从旧 stable-reference fixture 提取展示元数据，并替换为 Helix Host 可解析的 mediaInput。
async fn stage_media_node(
    node: &mut Value,
    gate: &str,
    slot: MediaSlot,
    ordinal: usize,
    staged_paths: &mut Vec<PathBuf>,
    staged: &mut Vec<StagedMedia>,
) -> Result<(), String> {
    let source = node
        .as_object()
        .ok_or_else(|| format!("{gate} media node must be object"))?;
    let name = source
        .get("name")
        .or_else(|| source.get("fileName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{gate} media node missing name"))?
        .to_string();
    let content_type = source
        .get("contentType")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{gate} media node missing contentType"))?
        .to_string();
    let size = source
        .get("size")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{gate} media node missing integer size"))?;
    let duration = source.get("duration").and_then(Value::as_f64);
    let width = source.get("width").and_then(Value::as_u64);
    let height = source.get("height").and_then(Value::as_u64);
    let bucket = match slot {
        MediaSlot::Files(_) if content_type.starts_with("image/") => "picture",
        MediaSlot::Files(_) if content_type.starts_with("video/") => "attachment",
        MediaSlot::Files(_) => {
            return Err(format!(
                "{gate} rich media contentType must be image/* or video/*"
            ))
        }
        MediaSlot::File => "attachment",
        MediaSlot::TemplateFile => "picture",
    };
    let (bytes, sha256) = media_fixture_bytes(size, &format!("{gate}:{ordinal}:{name}"))?;
    let local_path = media_fixture_path(gate, ordinal, &name);
    tokio::fs::write(&local_path, bytes)
        .await
        .map_err(|error| format!("{gate} write media fixture failed: {error}"))?;
    staged_paths.push(local_path.clone());

    let object = node
        .as_object_mut()
        .ok_or_else(|| format!("{gate} media node became non-object"))?;
    for field in [
        "bucket",
        "id",
        "uri",
        "sha256",
        "upload",
        "name",
        "fileName",
        "contentType",
        "size",
        "fileId",
        "objectKey",
        "headers",
        "url",
        "publicUrl",
        "method",
        "state",
        "etag",
    ] {
        object.remove(field);
    }
    object.insert(
        "mediaInput".to_string(),
        json!({
            "localPath": local_path.display().to_string(),
            "fileName": name.clone(),
            "contentType": content_type.clone(),
            "size": size,
            "sha256": sha256.clone(),
        }),
    );
    staged.push(StagedMedia {
        slot,
        name,
        content_type,
        size,
        sha256,
        bucket,
        duration,
        width,
        height,
        local_path,
    });
    Ok(())
}

/// 把媒体消息的 files/file/template.file 统一转成 Helix 上传状态机的输入，并负责失败回收。
async fn stage_media_inputs(props: Value, gate: &str) -> Result<(Value, Vec<StagedMedia>), String> {
    let mut staged_props = props;
    let mut staged_paths = Vec::new();
    let mut staged = Vec::new();
    let result = async {
        let has_files = staged_props
            .get("files")
            .and_then(Value::as_array)
            .is_some();
        let has_file = staged_props.get("file").is_some();
        let has_template_file = staged_props.pointer("/template/file").is_some();
        if [has_files, has_file, has_template_file]
            .into_iter()
            .filter(|present| *present)
            .count()
            != 1
        {
            return Err(format!(
                "{gate} media props must contain exactly one of files, file, template.file"
            ));
        }

        if let Some(files) = staged_props.get_mut("files").and_then(Value::as_array_mut) {
            if files.is_empty() {
                return Err(format!("{gate} media props.files must not be empty"));
            }
            for index in 0..files.len() {
                stage_media_node(
                    &mut files[index],
                    gate,
                    MediaSlot::Files(index),
                    index,
                    &mut staged_paths,
                    &mut staged,
                )
                .await?;
            }
        } else if let Some(file) = staged_props.get_mut("file") {
            stage_media_node(
                file,
                gate,
                MediaSlot::File,
                0,
                &mut staged_paths,
                &mut staged,
            )
            .await?;
        } else if let Some(file) = staged_props.pointer_mut("/template/file") {
            stage_media_node(
                file,
                gate,
                MediaSlot::TemplateFile,
                0,
                &mut staged_paths,
                &mut staged,
            )
            .await?;
        }
        Ok::<(), String>(())
    }
    .await;
    if let Err(error) = result {
        remove_staged_media_files(&staged_paths).await;
        return Err(error);
    }
    Ok((staged_props, staged))
}

/// 删除本轮生成的本地媒体字节；只接收 stage_media_inputs 返回的明确路径。
async fn remove_staged_media_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// 读取已上传消息中对应 slot 的稳定媒体节点。
fn staged_media_node<'a>(props: &'a Value, slot: MediaSlot) -> Option<&'a Value> {
    match slot {
        MediaSlot::Files(index) => props.get("files")?.as_array()?.get(index),
        MediaSlot::File => props.get("file"),
        MediaSlot::TemplateFile => props.pointer("/template/file"),
    }
}

/// 对账 Java/OSS/Helix 完成后的真实 fileId、uri、哈希和展示元数据，拒绝 fixture 回声冒充上传。
fn assert_staged_media_projection(
    actual_props: &Value,
    staged: &[StagedMedia],
    label: &str,
) -> Result<(), String> {
    for (index, fixture) in staged.iter().enumerate() {
        let node = staged_media_node(actual_props, fixture.slot)
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing"))?;
        let object = node
            .as_object()
            .ok_or_else(|| format!("{label} uploaded media slot {index} is not object"))?;
        let actual_bucket = string_field(node, &["bucket"])
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing bucket"))?;
        if actual_bucket != fixture.bucket {
            return Err(format!(
                "{label} uploaded media slot {index} bucket mismatch: expected {}, got {actual_bucket}",
                fixture.bucket
            ));
        }
        let file_id = string_field(node, &["id"])
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing Java fileId"))?;
        let uri = string_field(node, &["uri"])
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing stable uri"))?;
        let object_key = format!("media/{file_id}");
        if uri != object_key && uri != format!("/{object_key}") {
            return Err(format!(
                "{label} uploaded media slot {index} uri must equal objectKey (with optional leading slash), got {uri}"
            ));
        }
        let actual_name = string_field(node, &["name", "fileName"])
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing name"))?;
        if actual_name != fixture.name {
            return Err(format!(
                "{label} uploaded media slot {index} name mismatch: expected {}, got {actual_name}",
                fixture.name
            ));
        }
        let actual_content_type = string_field(node, &["contentType"])
            .ok_or_else(|| format!("{label} uploaded media slot {index} missing contentType"))?;
        if actual_content_type != fixture.content_type {
            return Err(format!(
                "{label} uploaded media slot {index} contentType mismatch: expected {}, got {actual_content_type}",
                fixture.content_type
            ));
        }
        if node.get("size").and_then(Value::as_u64) != Some(fixture.size) {
            return Err(format!(
                "{label} uploaded media slot {index} size mismatch: expected {}, got {}",
                fixture.size,
                node.get("size").unwrap_or(&Value::Null)
            ));
        }
        if string_field(node, &["sha256"]).as_deref() != Some(fixture.sha256.as_str()) {
            return Err(format!(
                "{label} uploaded media slot {index} sha256 does not match bytes written by Host"
            ));
        }
        if object
            .get("upload")
            .and_then(|upload| upload.get("status"))
            .and_then(Value::as_str)
            != Some("completed")
        {
            return Err(format!(
                "{label} uploaded media slot {index} did not reach upload.status=completed"
            ));
        }
        for (field, expected) in [
            ("duration", fixture.duration.map(Value::from)),
            ("width", fixture.width.map(Value::from)),
            ("height", fixture.height.map(Value::from)),
        ] {
            if let Some(expected) = expected {
                let matches = match (object.get(field), &expected) {
                    (Some(Value::Number(actual)), Value::Number(expected)) => {
                        actual.as_f64() == expected.as_f64()
                    }
                    (Some(actual), expected) => actual == expected,
                    (None, _) => false,
                };
                if !matches {
                    return Err(format!(
                        "{label} uploaded media slot {index} {field} mismatch: expected {expected}, got {}",
                        object.get(field).unwrap_or(&Value::Null)
                    ));
                }
            }
        }
    }
    Ok(())
}

/// 生成媒体证据摘要；不写入本地路径、票据 URL 或签名 headers。
fn staged_media_evidence(staged: &[StagedMedia]) -> Value {
    Value::Array(
        staged
            .iter()
            .map(|fixture| {
                json!({
                    "bucket": fixture.bucket,
                    "name": fixture.name,
                    "content_type": fixture.content_type,
                    "size": fixture.size,
                    "sha256": fixture.sha256,
                    "local_fixture": fixture.local_path.file_name().and_then(|name| name.to_str()),
                    "slot": format!("{:?}", fixture.slot),
                })
            })
            .collect(),
    )
}

pub(crate) struct SendEvidence {
    pub(crate) temporary_id: String,
    pub(crate) sending: DomainEventBytes,
    pub(crate) post_frame: Value,
    pub(crate) received: DomainEventBytes,
    pub(crate) row: Row,
    pub(crate) bus_envelope: Value,
}

/// 关闭一个本轮 Native WS，并把远端已先断开的竞态归一为幂等成功。
async fn close_send_transport(transport: Arc<NativeTransport>, label: &str) -> Result<(), String> {
    let result = timeout(Duration::from_secs(15), transport.close()).await;
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            let text = error.to_string();
            if [
                "closed connection",
                "connection is closed",
                "already closed",
                "Sending after closing is not allowed",
                "disconnected",
            ]
            .iter()
            .any(|marker| text.contains(marker))
            {
                Ok(())
            } else {
                Err(format!("close {label}: {error}"))
            }
        }
        Err(_) => Err(format!("close {label} timed out after 15s")),
    }
}

/// 持有 `run_helix_send` 已创建的资源；错误、超时和成功都必须经过同一收敛路径。
struct SendCleanup {
    storage: Option<NativeStorage>,
    db_path: PathBuf,
    observer: Option<RawWs>,
    actor_transport: Option<Arc<NativeTransport>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    engine: Option<JoinHandle<()>>,
}

impl SendCleanup {
    /// 创建只拥有本轮 SQLite 的清理上下文，后续资源按启动顺序逐项登记。
    fn new(storage: NativeStorage, db_path: PathBuf) -> Self {
        Self {
            storage: Some(storage),
            db_path,
            observer: None,
            actor_transport: None,
            shutdown_tx: None,
            engine: None,
        }
    }

    /// 借用本轮 Native SQLite；资源缺失时返回可诊断错误而不 panic。
    fn storage(&self) -> Result<&NativeStorage, String> {
        self.storage
            .as_ref()
            .ok_or_else(|| "send cleanup storage is not available".to_string())
    }

    /// 借用本轮 observer WS；等待路径不能绕过清理上下文取得旁路连接。
    fn observer_mut(&mut self) -> Result<&mut RawWs, String> {
        self.observer
            .as_mut()
            .ok_or_else(|| "send cleanup observer WS is not available".to_string())
    }

    /// 按 engine → actor WS → observer WS → SQLite 的顺序停止本轮资源并汇总失败。
    async fn close(mut self) -> Result<(), String> {
        let mut errors = Vec::new();
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(mut engine) = self.engine.take() {
            match timeout(Duration::from_secs(15), &mut engine).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("Helix engine task failed: {error}")),
                Err(_) => {
                    engine.abort();
                    match engine.await {
                        Ok(()) => errors.push(
                            "Helix engine graceful shutdown timed out after 15s; engine aborted"
                                .to_string(),
                        ),
                        Err(error) if error.is_cancelled() => errors.push(
                            "Helix engine graceful shutdown timed out after 15s; engine aborted"
                                .to_string(),
                        ),
                        Err(error) => errors.push(format!(
                            "Helix engine graceful shutdown timed out after 15s; engine abort join failed: {error}"
                        )),
                    }
                }
            }
        }
        if let Some(actor_transport) = self.actor_transport.take() {
            if let Err(error) = close_send_transport(actor_transport, "Helix actor WS").await {
                errors.push(error);
            }
        }
        if let Some(observer) = self.observer.take() {
            if let Err(error) = observer.close().await {
                errors.push(error);
            }
        }
        // Engine/transport 已停止后释放最后一个 SQLite Arc，避免 remove 只删主文件而 WAL 残留。
        drop(self.storage.take());
        remove_test_storage(&self.db_path);
        if self.db_path.exists()
            || PathBuf::from(format!("{}-wal", self.db_path.display())).exists()
            || PathBuf::from(format!("{}-shm", self.db_path.display())).exists()
        {
            errors.push(format!(
                "send cleanup left SQLite files at {}",
                self.db_path.display()
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// 将业务结果与本轮资源清理结果合并，既保留首个业务错误又不吞掉 cleanup 失败。
async fn finish_send_cleanup<T>(
    result: Result<T, String>,
    cleanup: SendCleanup,
) -> Result<T, String> {
    let cleanup_result = cleanup.close().await;
    match (result, cleanup_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(format!("send cleanup failed: {cleanup_error}")),
        (Err(error), Err(cleanup_error)) => {
            Err(format!("{error}; send cleanup failed: {cleanup_error}"))
        }
    }
}

/// 保留首个业务失败并合并两项清理失败；清理未完成时不能发布PASS。
pub(crate) fn finish_gate_cleanup<T>(
    result: Result<T, String>,
    session: Result<(), String>,
    channel: Result<(), String>,
) -> Result<T, String> {
    let mut errors = Vec::new();
    if let Err(error) = session {
        errors.push(format!("session cleanup failed: {error}"));
    }
    if let Err(error) = channel {
        errors.push(format!("channel cleanup failed: {error}"));
    }
    match (result, errors.is_empty()) {
        (Ok(value), true) => Ok(value),
        (Ok(_), false) => Err(errors.join("; ")),
        (Err(error), true) => Err(error),
        (Err(error), false) => Err(format!("{error}; {}", errors.join("; "))),
    }
}

/// 持有一次真实 Helix native engine 会话；所有 Phase2-4 command Gate 共用这一条边界。
///
/// 会话同时拥有 actor WS、NativeHttp、NativeStorage、NativeEventSink 和 engine task，
/// 因而测试不会把“直接 curl 成功”误报成 Helix 业务完成。调用方必须在断言前调用
/// [`RealHelixSession::command`]，并在返回前调用 [`RealHelixSession::close`]。
pub(crate) struct RealHelixSession {
    pub(crate) storage: NativeStorage,
    pub(crate) events: broadcast::Receiver<DomainEventBytes>,
    ws_events: broadcast::Receiver<Value>,
    tick_tx: mpsc::Sender<Tick>,
    actor_transport: Arc<NativeTransport>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    engine: Option<JoinHandle<()>>,
    raw_pump: Option<JoinHandle<()>>,
    db_path: PathBuf,
    /// 批量 Phase 模式下指向共享资源；句柄关闭只释放逻辑租约，不断开物理 WS。
    pool: Option<Arc<PooledSessionResource>>,
}

/// 一个 Phase 内按账号复用的真实 Helix engine/WS/SQLite 资源。
struct PooledSessionResource {
    storage: NativeStorage,
    event_sink: NativeEventSink,
    ws_events_tx: broadcast::Sender<Value>,
    tick_tx: mpsc::Sender<Tick>,
    actor_transport: Arc<NativeTransport>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
    engine: Mutex<Option<JoinHandle<()>>>,
    raw_pump: Mutex<Option<JoinHandle<()>>>,
    db_path: PathBuf,
    closed: AtomicBool,
}

/// 一个 Phase 的最多五个远端频道和按账号缓存的长连接。
struct PhaseResourcePool {
    http_base: Option<String>,
    ws_url: Option<String>,
    channels: Vec<String>,
    retired_channels: HashSet<String>,
    next_channel: usize,
    sessions: HashMap<String, Arc<PooledSessionResource>>,
}

static PHASE_RESOURCE_POOL: OnceCell<Mutex<PhaseResourcePool>> = OnceCell::const_new();
static PHASE_RUNTIME: OnceLock<tokio::runtime::Handle> = OnceLock::new();

impl PhaseResourcePool {
    /// 创建空的 Phase 资源池，首次真实 Gate 访问时按需预热频道。
    fn new() -> Self {
        Self {
            http_base: None,
            ws_url: None,
            channels: Vec::new(),
            retired_channels: HashSet::new(),
            next_channel: 0,
            sessions: HashMap::new(),
        }
    }
}

impl PooledSessionResource {
    /// 为同一账号建立独立事件订阅，但复用底层 engine、WS 和 SQLite。
    fn handle(self: &Arc<Self>) -> RealHelixSession {
        RealHelixSession {
            storage: self.storage.clone(),
            events: self.event_sink.subscribe(),
            ws_events: self.ws_events_tx.subscribe(),
            tick_tx: self.tick_tx.clone(),
            actor_transport: Arc::clone(&self.actor_transport),
            shutdown_tx: None,
            engine: None,
            raw_pump: None,
            db_path: self.db_path.clone(),
            pool: Some(Arc::clone(self)),
        }
    }

    /// 停止 Phase 共享 engine/WS；幂等保护避免 cleanup test 重复关闭。
    async fn shutdown(&self) -> Result<(), String> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let mut errors = Vec::new();
        if let Some(shutdown_tx) = self.shutdown_tx.lock().await.take() {
            let _ = shutdown_tx.send(());
        }
        let raw_pump = self.raw_pump.lock().await.take();
        stop_raw_pump(raw_pump).await;
        if let Some(mut engine) = self.engine.lock().await.take() {
            match timeout(Duration::from_secs(15), &mut engine).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => errors.push(format!("Phase pooled engine task failed: {error}")),
                Err(_) => {
                    engine.abort();
                    let _ = engine.await;
                    errors.push(
                        "Phase pooled engine graceful shutdown timed out after 15s; engine aborted"
                            .to_string(),
                    );
                }
            }
        }
        if let Err(error) = close_send_transport(
            Arc::clone(&self.actor_transport),
            "Phase pooled Helix actor WS",
        )
        .await
        {
            errors.push(error);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}

/// 判断当前 Cargo 批次是否启用 Phase 资源复用；单 Gate 调试默认保持旧隔离语义。
fn phase_pool_enabled() -> bool {
    matches!(
        std::env::var("MESSAGEV3_PHASE_POOL")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes")
    )
}

/// 读取可配置的 WS 心跳间隔，限制在合理范围内避免测试连接无心跳或过度打点。
fn messagev3_ping_interval_ms() -> u64 {
    std::env::var("MESSAGEV3_WS_PING_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1_000..=300_000).contains(value))
        .unwrap_or(PHASE_POOL_PING_INTERVAL_DEFAULT_MS)
}

/// 返回本进程的 Phase 资源池；未启用复用时调用方不会触碰该状态。
async fn phase_resource_pool() -> &'static Mutex<PhaseResourcePool> {
    PHASE_RESOURCE_POOL
        .get_or_init(|| async { Mutex::new(PhaseResourcePool::new()) })
        .await
}

impl RealHelixSession {
    /// 暴露本轮数据库位置，供关闭失败后的清理核验保留资源身份。
    pub(crate) fn storage_path(&self) -> &Path {
        &self.db_path
    }

    /// 判断当前句柄是否只持有 Phase 共享资源的逻辑租约。
    pub(crate) fn is_pooled(&self) -> bool {
        self.pool.is_some()
    }

    /// 向当前 engine 注入一条真实 `Tick::Command`，不绕过 Helix public module。
    pub(crate) async fn command(&self, name: &'static str, payload: Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&payload)
            .map_err(|error| format!("serialize {name} payload: {error}"))?;
        self.tick_tx
            .send(Tick::Command(AppCommand::new(name, Bytes::from(bytes))))
            .await
            .map_err(|error| format!("send {name} Tick failed: {error}"))
    }

    /// 等待当前会话的 canonical DomainEvent，并保留精确 data predicate。
    pub(crate) async fn wait_event<F>(
        &mut self,
        event_name: &str,
        predicate: F,
        wait_for: Duration,
    ) -> Result<DomainEventBytes, String>
    where
        F: Fn(&Value) -> bool,
    {
        wait_domain_event(&mut self.events, event_name, predicate, wait_for).await
    }

    /// 等待同一 Native WS 入站泵旁路记录的原始 Go JSON，不创建竞争性的第二条连接。
    pub(crate) async fn wait_ws_action<F>(
        &mut self,
        action: &str,
        predicate: F,
        wait_for: Duration,
    ) -> Result<Value, String>
    where
        F: Fn(&Value) -> bool,
    {
        timeout(wait_for, async {
            loop {
                let frame = self
                    .ws_events
                    .recv()
                    .await
                    .map_err(|error| format!("raw Go WS tap receive error: {error}"))?;
                if frame.get("action").and_then(Value::as_str) == Some(action) && predicate(&frame)
                {
                    return Ok(frame);
                }
            }
        })
        .await
        .map_err(|_| format!("timed out waiting tapped Go WS action '{action}'"))?
    }

    /// 关闭逻辑租约；Phase 池句柄保留物理 WS，批次末由 cleanup test 统一回收。
    pub(crate) async fn close(self) -> Result<(), String> {
        if let Some(pool) = self.pool {
            if pool.closed.load(Ordering::Acquire) {
                return Err("Phase pooled Helix session was already closed".to_string());
            }
            return Ok(());
        }
        close_session_parts(
            self.shutdown_tx,
            self.engine,
            self.actor_transport,
            self.raw_pump,
            self.db_path,
            true,
        )
        .await
    }

    /// 关闭 engine/WS 但保留 SQLite，供重启后的只读回读验收使用。
    pub(crate) async fn close_preserving_db(self) -> Result<PathBuf, String> {
        let db_path = self.db_path.clone();
        if let Some(pool) = self.pool {
            shutdown_pooled_resource_on_phase_runtime(Arc::clone(&pool)).await?;
            // 重开验收拥有该物理资源；从池移除最后的缓存租约后再交还数据库路径。
            phase_resource_pool()
                .await
                .lock()
                .await
                .sessions
                .retain(|_, resource| !Arc::ptr_eq(resource, &pool));
            return Ok(db_path);
        }
        close_session_parts(
            self.shutdown_tx,
            self.engine,
            self.actor_transport,
            self.raw_pump,
            db_path.clone(),
            false,
        )
        .await?;
        Ok(db_path)
    }
}

/// 统一 raw WS 与 Phase pooled session 的观察接口，避免 pooled 批次再开第二条竞争连接。
enum WsTap {
    Pooled(RealHelixSession),
    Raw(RawWs),
}

impl WsTap {
    /// 确认观察连接可用；pooled session 的 authenticated hello 在物理连接创建时已核验。
    async fn ensure_ready(&mut self) -> Result<(), String> {
        match self {
            Self::Pooled(session) => session
                .pool
                .as_ref()
                .filter(|resource| !resource.closed.load(Ordering::Acquire))
                .map(|_| ())
                .ok_or_else(|| "pooled WS resource is closed".to_string()),
            Self::Raw(ws) => wait_ws_hello(ws).await.map(|_| ()),
        }
    }

    /// 从观察连接读取指定 action，并保留真实 Go 帧的 predicate 语义。
    async fn wait_action<F>(
        &mut self,
        action: &str,
        predicate: F,
        wait_for: Duration,
    ) -> Result<Value, String>
    where
        F: Fn(&Value) -> bool,
    {
        match self {
            Self::Pooled(session) => session.wait_ws_action(action, predicate, wait_for).await,
            Self::Raw(ws) => wait_ws_action(ws, action, predicate, wait_for).await,
        }
    }

    /// 释放观察连接的逻辑租约；pooled 物理 WS 延迟至 Phase cleanup。
    async fn close(self) -> Result<(), String> {
        match self {
            Self::Pooled(session) => session.close().await,
            Self::Raw(ws) => ws.close().await,
        }
    }
}

/// 统一收敛 engine 超时、迟到断连和 SQLite 清理，避免早退留下 reconnect supervisor。
async fn close_session_parts(
    shutdown_tx: Option<oneshot::Sender<()>>,
    engine: Option<JoinHandle<()>>,
    actor_transport: Arc<NativeTransport>,
    raw_pump: Option<JoinHandle<()>>,
    db_path: PathBuf,
    remove_db: bool,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Some(shutdown_tx) = shutdown_tx {
        let _ = shutdown_tx.send(());
    }
    // 先停止 transport→engine 的旁路泵，避免关闭阶段继续转发迟到帧或阻塞 Disconnected 入队。
    stop_raw_pump(raw_pump).await;
    if let Some(mut engine) = engine {
        match timeout(Duration::from_secs(15), &mut engine).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => errors.push(format!("real Helix engine task failed: {error}")),
            Err(_) => {
                engine.abort();
                let _ = engine.await;
                errors.push(
                    "real Helix engine graceful shutdown timed out after 15s; engine aborted"
                        .to_string(),
                );
            }
        }
    }
    // Go may close the socket after the terminal projection; cleanup remains idempotent.
    if let Err(error) = close_send_transport(actor_transport, "real Helix actor WS").await {
        errors.push(error);
    }
    if remove_db {
        remove_test_storage(&db_path);
        if db_path.exists()
            || PathBuf::from(format!("{}-wal", db_path.display())).exists()
            || PathBuf::from(format!("{}-shm", db_path.display())).exists()
        {
            errors.push(format!(
                "real Helix SQLite cleanup left files at {}",
                db_path.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// 终止并 join raw WS 入站泵，保证 session/Phase cleanup 后不遗留 detached Tokio task。
async fn stop_raw_pump(raw_pump: Option<JoinHandle<()>>) {
    if let Some(raw_pump) = raw_pump {
        raw_pump.abort();
        let _ = raw_pump.await;
    }
}

/// 返回一个跨 `#[tokio::test]` 生命周期的专用运行时，托管 Phase 共享 WS/engine 任务。
fn phase_runtime_handle() -> &'static tokio::runtime::Handle {
    PHASE_RUNTIME.get_or_init(|| {
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        thread::Builder::new()
            .name("messagev3-phase-runtime".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("messagev3 phase runtime must initialize");
                ready_tx
                    .send(runtime.handle().clone())
                    .expect("messagev3 phase runtime receiver must remain alive");
                runtime.block_on(std::future::pending::<()>());
            })
            .expect("messagev3 phase runtime thread must start");
        ready_rx
            .recv()
            .expect("messagev3 phase runtime must report ready")
    })
}

/// 把共享资源启动/停止任务投递到持久运行时，避免测试函数结束时 Tokio 回收 WS reader。
async fn run_on_phase_runtime<F, T>(future: F) -> Result<T, String>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let (result_tx, result_rx) = oneshot::channel();
    let task = phase_runtime_handle().spawn(async move {
        let _ = result_tx.send(future.await);
    });
    match timeout(
        Duration::from_secs(PHASE_RUNTIME_TASK_TIMEOUT_SECS),
        result_rx,
    )
    .await
    {
        Ok(result) => result.map_err(|_| "messagev3 phase runtime task was cancelled".to_string()),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(format!(
                "messagev3 phase runtime task timed out after {PHASE_RUNTIME_TASK_TIMEOUT_SECS}s"
            ))
        }
    }
}

/// 在共享运行时关闭一个 pooled engine/WS，并把业务错误原样回传给 Phase cleanup。
async fn shutdown_pooled_resource_on_phase_runtime(
    resource: Arc<PooledSessionResource>,
) -> Result<(), String> {
    run_on_phase_runtime(async move { resource.shutdown().await }).await?
}

/// 启动一条注册指定频道的真实 Helix native engine 会话，并等待 authenticated hello。
pub(crate) async fn start_real_helix_session(
    context: &AuthContext,
    channel_ids: &[&str],
) -> Result<RealHelixSession, String> {
    start_real_helix_session_as(context, &context.actor, channel_ids).await
}

/// 启动以指定真实登录身份连接 Go WS 的 native engine，用于双用户 Gate 的 author/reader 断言。
pub(crate) async fn start_real_helix_session_as(
    context: &AuthContext,
    persona: &Persona,
    channel_ids: &[&str],
) -> Result<RealHelixSession, String> {
    start_real_helix_session_as_at(context, persona, channel_ids, go_http_base(), go_ws_url()).await
}

/// 启动 Phase5 文字接龙 native engine，端点由当前 profile 提供。
pub(crate) async fn start_chain_helix_session(
    context: &AuthContext,
    channel_ids: &[&str],
) -> Result<RealHelixSession, String> {
    start_chain_helix_session_as(context, &context.actor, channel_ids).await
}

/// 以指定登录身份启动文字接龙 native engine，供双设备同步 Gate 使用。
pub(crate) async fn start_chain_helix_session_as(
    context: &AuthContext,
    persona: &Persona,
    channel_ids: &[&str],
) -> Result<RealHelixSession, String> {
    start_real_helix_session_as_at(
        context,
        persona,
        channel_ids,
        chain_go_http_base(),
        chain_go_ws_url(),
    )
    .await
}

/// 保存一次 engine 装配结果，既可返回隔离句柄，也可放入 Phase 共享池。
struct SessionComponents {
    storage: NativeStorage,
    event_sink: NativeEventSink,
    events: broadcast::Receiver<DomainEventBytes>,
    ws_events_tx: broadcast::Sender<Value>,
    ws_events: broadcast::Receiver<Value>,
    tick_tx: mpsc::Sender<Tick>,
    actor_transport: Arc<NativeTransport>,
    shutdown_tx: oneshot::Sender<()>,
    engine: JoinHandle<()>,
    raw_pump: JoinHandle<()>,
    db_path: PathBuf,
}

/// 为真实链的延迟断言创建有界观察队列；不改变产品 EventSink 默认容量。
fn new_session_event_sink() -> (NativeEventSink, broadcast::Receiver<DomainEventBytes>) {
    NativeEventSink::new_with_capacity(PHASE_POOL_EVENT_CAPACITY)
}

/// 按给定 Go HTTP/WS 端点装配完整 Native 生命周期；业务仍只在 helix-im 内执行。
async fn start_session_components(
    persona: &Persona,
    channel_ids: &[&str],
    http_base: &str,
    ws_url: &str,
) -> Result<SessionComponents, String> {
    let metric_runtime = metrics::start();
    let metric_sink = metric_runtime.sink();
    let (storage, db_path) = open_test_storage().await?;
    let storage = storage.with_metric_sink(Arc::clone(&metric_sink));
    let (event_sink, events) = new_session_event_sink();
    let event_sink = event_sink.with_metric_sink(Arc::clone(&metric_sink));
    let http =
        go_http_at_with_metrics(persona, http_base, ws_url, Arc::clone(&metric_sink)).await?;

    let mut im_config = ImConfig::default();
    im_config.ws_url = ws_url.to_string();
    im_config.api_base_url = http_base.to_string();
    im_config.default_api_base_url = main_api_base();
    im_config.auth_user_id = persona.user_id.clone();
    im_config.company_id = persona.company_id.clone();
    im_config.user_name = persona.user_name.clone();
    im_config.org_name = persona.org_name.clone();
    im_config.dept_name = persona.dept_name.clone();
    im_config.ping_interval_ms = messagev3_ping_interval_ms();
    im_config.send_timeout_ms = 15_000;

    let mut module = ImModule::new(im_config);
    for channel_id in channel_ids {
        let channel = helix_im::state::ChannelId::from_str(channel_id)
            .ok_or_else(|| format!("channel id is not canonical Helix ChannelId: {channel_id}"))?;
        module.register_channel(channel, 0);
    }
    let mut shell = ExecutionShell::new();
    shell.register(module);

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let (transport_tick_tx, mut transport_tick_rx) = mpsc::channel::<Tick>(256);
    let (ws_events_tx, ws_events_rx) = broadcast::channel::<Value>(4096);
    let ws_events_pump_tx = ws_events_tx.clone();
    let engine_tick_tx = tick_tx.clone();
    // 记录同一真实 transport 的 raw JSON 后再转交 engine，避免额外 WS 连接抢走定向帧。
    let raw_pump = tokio::spawn(async move {
        while let Some(tick) = transport_tick_rx.recv().await {
            if let Tick::Inbound(bytes) = &tick {
                if let Ok(frame) = serde_json::from_slice::<Value>(bytes.as_bytes()) {
                    let _ = ws_events_pump_tx.send(frame);
                }
            }
            if engine_tick_tx.send(tick).await.is_err() {
                break;
            }
        }
    });
    let transport_raw =
        100u32.saturating_add((RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 10_000) as u32);
    let mut actor_transport = NativeTransport::new(
        ws_url,
        TransportId::from_raw(transport_raw),
        Some(transport_tick_tx),
    )
    .with_handshake_headers(ws_headers(persona))
    .with_metric_sink(Arc::clone(&metric_sink));
    let activation = match timeout(
        Duration::from_secs(SESSION_SETUP_TIMEOUT_SECS),
        actor_transport.connect(),
    )
    .await
    {
        Ok(Ok(activation)) => activation,
        Ok(Err(error)) => {
            stop_raw_pump(Some(raw_pump)).await;
            return Err(format!("real Helix actor WS connect failed: {error}"));
        }
        Err(_) => {
            stop_raw_pump(Some(raw_pump)).await;
            return Err(format!(
                "real Helix actor WS connect timed out after {SESSION_SETUP_TIMEOUT_SECS}s"
            ));
        }
    };
    let actor_transport = Arc::new(actor_transport);
    let mut transports = TransportTable::new();
    transports.insert(
        TransportId::from_raw(transport_raw),
        Arc::clone(&actor_transport),
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let engine_tick_tx = tick_tx.clone();
    let engine_storage = storage.clone();
    let engine_event_sink = event_sink.clone();
    let engine = tokio::spawn(async move {
        run_engine_loop_with_transports(
            shell,
            tick_rx,
            engine_tick_tx,
            EngineConfig::new(
                engine_storage,
                http,
                SharedFileUploader::default(),
                engine_event_sink,
                8,
            )
            .with_metric_sink(metric_sink),
            shutdown_rx,
            transports,
        )
        .await;
        metrics::shutdown(metric_runtime).await;
    });
    let activation_result = timeout(
        Duration::from_secs(SESSION_SETUP_TIMEOUT_SECS),
        activation.activate(),
    )
    .await;
    let activation_error = match activation_result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(format!("real Helix actor WS activation failed: {error}")),
        Err(_) => Some(format!(
            "real Helix actor WS activation timed out after {SESSION_SETUP_TIMEOUT_SECS}s"
        )),
    };
    if let Some(activation_error) = activation_error {
        let cleanup = close_session_parts(
            Some(shutdown_tx),
            Some(engine),
            Arc::clone(&actor_transport),
            Some(raw_pump),
            db_path,
            true,
        )
        .await;
        return Err(match cleanup {
            Ok(()) => activation_error,
            Err(cleanup_error) => format!("{activation_error}; cleanup failed: {cleanup_error}"),
        });
    }

    Ok(SessionComponents {
        storage,
        event_sink,
        events,
        ws_events_tx,
        ws_events: ws_events_rx,
        tick_tx,
        actor_transport,
        shutdown_tx,
        engine,
        raw_pump,
        db_path,
    })
}

/// 等待一次共享 session 的 authenticated hello，防止后续 Gate 吃到连接前帧。
async fn wait_session_ready(
    events: &mut broadcast::Receiver<DomainEventBytes>,
) -> Result<(), String> {
    wait_domain_event(
        events,
        "im:connection:established",
        |_| true,
        Duration::from_secs(20),
    )
    .await
    .map(|_| ())
}

/// 首次创建 pooled session 时等待一次恢复终态，后续 Gate 复用该已认证/已恢复的长连接。
async fn wait_pooled_session_ready(
    events: &mut broadcast::Receiver<DomainEventBytes>,
) -> Result<(), String> {
    wait_session_ready(events).await?;
    wait_domain_event(
        events,
        "im:sync:recovered",
        |_| true,
        Duration::from_secs(20),
    )
    .await
    .map(|_| ())
}

/// 将隔离 engine 组件包装成保持既有调用方字段语义的句柄。
fn isolated_session(components: SessionComponents) -> RealHelixSession {
    RealHelixSession {
        storage: components.storage,
        events: components.events,
        ws_events: components.ws_events,
        tick_tx: components.tick_tx,
        actor_transport: components.actor_transport,
        shutdown_tx: Some(components.shutdown_tx),
        engine: Some(components.engine),
        raw_pump: Some(components.raw_pump),
        db_path: components.db_path,
        pool: None,
    }
}

/// 在 Phase 池中按账号取得一个长连接句柄，并为每个句柄订阅独立事件游标。
async fn pooled_session_at(
    context: &AuthContext,
    persona: &Persona,
    http_base: &str,
    ws_url: &str,
) -> Result<RealHelixSession, String> {
    let channels = phase_pool_channels_at(context, http_base, ws_url).await?;
    let key = format!(
        "{}:{}:{}:{}",
        persona.user_id, persona.company_id, http_base, ws_url
    );
    let pool = phase_resource_pool().await;
    let mut state = pool.lock().await;
    if let Some(resource) = state.sessions.get(&key) {
        if !resource.closed.load(Ordering::Acquire) {
            return Ok(resource.handle());
        }
    }
    state.sessions.remove(&key);
    let phase_persona = persona.clone();
    let phase_http_base = http_base.to_string();
    let phase_ws_url = ws_url.to_string();
    let phase_channels = channels.clone();
    let mut components = run_on_phase_runtime(async move {
        let channel_refs = phase_channels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        start_session_components(
            &phase_persona,
            &channel_refs,
            &phase_http_base,
            &phase_ws_url,
        )
        .await
    })
    .await??;
    if let Err(error) = wait_pooled_session_ready(&mut components.events).await {
        let cleanup = run_on_phase_runtime(async move {
            close_session_parts(
                Some(components.shutdown_tx),
                Some(components.engine),
                components.actor_transport,
                Some(components.raw_pump),
                components.db_path,
                true,
            )
            .await
        })
        .await;
        return Err(match cleanup {
            Ok(Ok(())) => format!("pooled session readiness failed: {error}"),
            Ok(Err(cleanup_error)) => {
                format!("pooled session readiness failed: {error}; cleanup failed: {cleanup_error}")
            }
            Err(cleanup_error) => {
                format!("pooled session readiness failed: {error}; cleanup dispatch failed: {cleanup_error}")
            }
        });
    }
    let resource = Arc::new(PooledSessionResource {
        storage: components.storage,
        event_sink: components.event_sink,
        ws_events_tx: components.ws_events_tx,
        tick_tx: components.tick_tx,
        actor_transport: components.actor_transport,
        shutdown_tx: Mutex::new(Some(components.shutdown_tx)),
        engine: Mutex::new(Some(components.engine)),
        raw_pump: Mutex::new(Some(components.raw_pump)),
        db_path: components.db_path,
        closed: AtomicBool::new(false),
    });
    state.sessions.insert(key, Arc::clone(&resource));
    Ok(resource.handle())
}

/// 根据当前批次开关选择隔离 session 或按账号复用的 Phase session。
async fn start_real_helix_session_as_at(
    context: &AuthContext,
    persona: &Persona,
    channel_ids: &[&str],
    http_base: &str,
    ws_url: &str,
) -> Result<RealHelixSession, String> {
    if phase_pool_enabled() {
        return pooled_session_at(context, persona, http_base, ws_url).await;
    }
    let mut components = start_session_components(persona, channel_ids, http_base, ws_url).await?;
    if let Err(error) = wait_session_ready(&mut components.events).await {
        let cleanup = close_session_parts(
            Some(components.shutdown_tx),
            Some(components.engine),
            components.actor_transport,
            Some(components.raw_pump),
            components.db_path,
            true,
        )
        .await;
        return Err(match cleanup {
            Ok(()) => format!("real Helix session readiness failed: {error}"),
            Err(cleanup_error) => {
                format!(
                    "real Helix session readiness failed: {error}; cleanup failed: {cleanup_error}"
                )
            }
        });
    }
    Ok(isolated_session(components))
}

/// 生成不含账号/令牌的本轮 correlation 头和 run-owned 资源前缀。
pub(crate) fn run_token(prefix: &str) -> String {
    let seq = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("hx-p1-{prefix}-{}-{seq}", std::process::id())
}

/// 返回当前测试使用的唯一远端资源前缀；只允许清理带该前缀的资源。
pub(crate) fn resource_name(domain: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("hx-p1-{domain}-{}-{millis}", std::process::id())
}

/// 在单进程内串行化远端业务写，避免测试并行时互相消费 WS 广播或重用频道。
pub(crate) async fn remote_setup() -> (tokio::sync::MutexGuard<'static, ()>, Arc<AuthContext>) {
    let guard = REMOTE_LOCK
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    let context = AUTH_CONTEXT
        .get_or_init(|| async { discover_auth_context().await.map(Arc::new) })
        .await
        .clone()
        .unwrap_or_else(|error| panic!("BLOCKED_PRECONDITION/NOT_VERIFIED: {error}"));
    (guard, context)
}

/// 为 Search Gate 获取已通过只读 ES 探针的会话；普通 Gate 不被 Search 权限波动拖住。
pub(crate) async fn remote_setup_search(
) -> (tokio::sync::MutexGuard<'static, ()>, Arc<SearchFixture>) {
    let guard = REMOTE_LOCK
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    let context = SEARCH_AUTH_CONTEXT
        .get_or_init(|| async {
            discover_search_auth_context_at(go_http_base(), go_health_url())
                .await
                .map(Arc::new)
        })
        .await
        .clone()
        .unwrap_or_else(|error| panic!("BLOCKED_PRECONDITION/NOT_VERIFIED: {error}"));
    let fixture = SEARCH_FIXTURE
        .get_or_init(|| {
            let context = Arc::clone(&context);
            async move {
                let seed =
                    search_observer_preflight(&context.observer, go_http_base(), go_health_url())
                        .await
                        .ok_or_else(|| {
                            "selected Search observer no longer has a non-empty real search fixture"
                                .to_string()
                        })?;
                Ok(Arc::new(SearchFixture {
                    context,
                    channel_id: seed.channel_id,
                    post_id: seed.post_id,
                    keyword: SEARCH_FIXTURE_KEYWORD.to_string(),
                }))
            }
        })
        .await
        .clone()
        .unwrap_or_else(|error| panic!("BLOCKED_PRECONDITION/NOT_VERIFIED: {error}"));
    (guard, fixture)
}

/// 串行化并准备 Phase5 的 profile 真实账号上下文。
pub(crate) async fn remote_setup_chain() -> (tokio::sync::MutexGuard<'static, ()>, Arc<AuthContext>)
{
    let guard = REMOTE_LOCK
        .get_or_init(|| async { Mutex::new(()) })
        .await
        .lock()
        .await;
    let context = CHAIN_AUTH_CONTEXT
        .get_or_init(|| async {
            discover_auth_context_at(chain_go_http_base(), go_health_url())
                .await
                .map(Arc::new)
        })
        .await
        .clone()
        .unwrap_or_else(|error| panic!("BLOCKED_PRECONDITION/NOT_VERIFIED: {error}"));
    (guard, context)
}

/// 读取 CI File 变量路径，未配置时回退到现有 Angular workspace 凭据。
pub(crate) fn discover_credentials_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CSES_WDIO_CREDENTIALS_FILE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "CSES_WDIO_CREDENTIALS_FILE does not point to a readable file: {}",
            path.display()
        ));
    }
    let mut candidates = vec![PathBuf::from(
        "/System/Volumes/Data/workspace/angular/visual/cses-client/.wdio/credentials.local.json",
    )];
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest.ancestors() {
        candidates.push(ancestor.join(CREDENTIALS_RELATIVE));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "credentials.local.json not found at the existing Angular workspace path; +             install/use the existing local account pool before running real Phase1 tests"
                .to_string()
        })
}

/// 校验内联账号池非空，避免 CI 把空 Masked 变量静默回退到开发机文件。
fn validate_inline_credentials(content: Option<String>) -> Result<Option<String>, String> {
    match content {
        Some(content) if content.trim().is_empty() => {
            Err("CSES_WDIO_CREDENTIALS_JSON is configured but empty".to_string())
        }
        content => Ok(content),
    }
}

/// 优先读取 Masked JSON 账号池，并保留 GitLab File 与本地文件兼容入口。
fn load_credentials_content() -> Result<String, String> {
    let inline = match std::env::var("CSES_WDIO_CREDENTIALS_JSON") {
        Ok(content) => Some(content),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("CSES_WDIO_CREDENTIALS_JSON must be valid UTF-8".to_string());
        }
    };
    if let Some(content) = validate_inline_credentials(inline)? {
        return Ok(content);
    }
    let credentials_path = discover_credentials_path()?;
    std::fs::read_to_string(&credentials_path)
        .map_err(|error| format!("read credentials file failed: {error}"))
}

/// 创建一个没有身份头的 HTTP client，用于 Go health 或主登录接口。
pub(crate) async fn anonymous_http(base: &str) -> Result<NativeHttp, String> {
    anonymous_http_at(base, go_ws_url()).await
}

/// 构造不带身份头的 HTTP client，并显式绑定对应 WS 端点。
pub(crate) async fn anonymous_http_at(base: &str, ws_url: &str) -> Result<NativeHttp, String> {
    NativeHttp::with_config(HostNetworkConfig::new(base, ws_url))
        .map_err(|error| format!("NativeHttp init failed for {base}: {error}"))
}

/// 发送 JSON 请求并保留业务信封；HTTP 200 不在此处等价于业务 SUCCESS。
pub(crate) async fn request_json(
    http: &NativeHttp,
    method: &str,
    url: &str,
    body: Option<Value>,
    track_prefix: &str,
) -> Result<HttpObservation, String> {
    let request = HttpRequest {
        method: method.to_string(),
        url: url.to_string(),
        headers: vec![
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Cses-Track-Id".to_string(), run_token(track_prefix)),
        ],
        body: body
            .map(|value| serde_json::to_vec(&value).map(Bytes::from))
            .transpose()
            .map_err(|error| format!("serialize JSON request: {error}"))?,
    };
    // 为测试端的 HTTP 观测加总时限，服务端半关闭或连接进入 CLOSE_WAIT 时不能让整批 Gate 永久悬挂。
    let response = timeout(Duration::from_secs(35), http.request(request))
        .await
        .map_err(|_| format!("{method} {url} transport timeout after 35s"))?
        .map_err(|error| format!("{method} {url} transport error: {error}"))?;
    let body = if response.body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(response.body.as_ref())
            .map_err(|error| format!("{method} {url} returned invalid JSON: {error}"))?
    };
    Ok(HttpObservation {
        status: response.status,
        body,
    })
}

/// 从真实 Go CommonRes 中要求业务成功；失败信息只带业务 message，不泄露凭据。
pub(crate) fn require_success(
    observation: HttpObservation,
    operation: &str,
) -> Result<Value, String> {
    if observation.status != 200 {
        return Err(format!(
            "{operation} returned HTTP {}, body={}",
            observation.status, observation.body
        ));
    }
    if observation.body.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        return Err(format!(
            "{operation} returned business failure: status={}, message={}",
            observation.body.get("status").unwrap_or(&Value::Null),
            observation.body.get("message").unwrap_or(&Value::Null)
        ));
    }
    Ok(observation.body)
}

/// 从 JSON 中读取字符串字段，兼容真实登录接口将 ID 编码为 number 的历史返回。
pub(crate) fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    })
}

/// 从登录响应提取某租户的展示字段和所有可用 companyId。
pub(crate) fn persona_from_login(
    value: &Value,
    role: &'static str,
    mobile: &str,
) -> Result<Persona, String> {
    let user_id = string_field(value, &["userId", "user_id", "id"])
        .ok_or_else(|| "login response missing userId".to_string())?;
    let cookie_id = string_field(value, &["cookieId", "cookieid", "cookie_id"])
        .unwrap_or_else(|| user_id.clone());
    if cookie_id != user_id {
        return Err(format!(
            "login response cookieId/userId mismatch for role {role}; refusing ambiguous session"
        ));
    }
    let mut companies = BTreeSet::new();
    if let Some(company) = string_field(value, &["companyId", "company_id", "teamId", "team_id"]) {
        companies.insert(company);
    }
    let mut user_name = string_field(value, &["userName", "username", "nickname", "name"])
        .unwrap_or_else(|| mobile.to_string());
    let mut org_name = String::new();
    let mut dept_name = String::new();
    if let Some(organizes) = value.get("organizes").and_then(Value::as_array) {
        for organize in organizes {
            if let Some(company) = string_field(organize, &["companyId", "company_id", "teamId"]) {
                companies.insert(company);
            }
            if user_name == mobile {
                user_name = string_field(organize, &["userName", "username", "name"])
                    .unwrap_or_else(|| user_name.clone());
            }
            if org_name.is_empty() {
                org_name =
                    string_field(organize, &["companyName", "company_name"]).unwrap_or_default();
            }
            if dept_name.is_empty() {
                dept_name = string_field(organize, &["departmentName", "department_name"])
                    .unwrap_or_default();
            }
        }
    }
    if companies.is_empty() {
        return Err(format!("login response has no companyId for role {role}"));
    }
    Ok(Persona {
        user_id,
        cookie_id,
        company_id: String::new(),
        user_name,
        org_name,
        dept_name,
        companies: companies.into_iter().collect(),
    })
}

/// 登录一个账号；密码只进入本次 HTTP body，不写日志、测试 artifact 或 panic 文本。
pub(crate) async fn login_account(
    account: &CredentialAccount,
    role: &'static str,
) -> Result<Persona, String> {
    let mobile = account
        .mobile
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "credential account has no mobile; skipping unusable account".to_string())?;
    let main_api_base = main_api_base();
    let http = anonymous_http(&main_api_base).await?;
    for (name, value) in [
        ("appType", "bct"),
        ("device", "IOS"),
        ("language", "zh"),
        ("appVersion", "1.0.120"),
        ("timeZone", "Asia/Shanghai"),
        ("osVersion", "16.5.1"),
        ("siteName", "b"),
        ("envType", "pre"),
        ("deviceId", "helix-phase1-rust"),
    ] {
        http.set_global_header(name, value)
            .await
            .map_err(|error| format!("set login header {name}: {error}"))?;
    }
    let observation = request_json(
        &http,
        "POST",
        &format!("{main_api_base}/User/login"),
        Some(json!({ "userName": mobile, "password": account.password })),
        "login",
    )
    .await?;
    if observation.status != 200 {
        return Err(format!("login HTTP status {}", observation.status));
    }
    persona_from_login(&observation.body, role, mobile)
}

/// 从 Go/ES 的真实 `search/do` 响应提取可复用的频道和帖子身份，不写入共享索引。
fn extract_search_fixture_seed(body: &Value) -> Option<SearchFixtureSeed> {
    let groups = body
        .pointer("/data/messageGroups/items")
        .and_then(Value::as_array)?;
    groups.iter().find_map(|group| {
        let channel_id = group
            .get("channel")
            .and_then(|channel| string_field(channel, &["id", "channelId", "channel_id"]))
            .or_else(|| string_field(group, &["channelId", "channel_id"]))
            .or_else(|| {
                group
                    .pointer("/posts/0")
                    .and_then(|post| string_field(post, &["channelId", "channel_id"]))
            })?;
        let post_id = group
            .pointer("/posts/0")
            .and_then(|post| string_field(post, &["id", "postId", "post_id"]))?;
        Some(SearchFixtureSeed {
            channel_id,
            post_id,
        })
    })
}

/// 只读探测候选账号是否拥有当前 PRE Search 的 ES actor 范围，并返回动态 fixture。
///
/// Search 的 actor 文档由 Go/ES 维护，不能从频道 WS hello 推断；探针只发一个
/// `search/do` 读请求，并要求 users/channels/messageGroups 三个分页均为非空 SUCCESS。
async fn search_observer_preflight(
    persona: &Persona,
    go_http_base: &str,
    go_health_url: &str,
) -> Option<SearchFixtureSeed> {
    let ws_url = go_health_base_to_ws(go_health_url);
    let http = match go_http_at(persona, go_http_base, ws_url).await {
        Ok(http) => http,
        Err(_) => return None,
    };
    let observation = match timeout(
        Duration::from_secs(SEARCH_PREFLIGHT_TIMEOUT_SECS),
        request_json(
            &http,
            "POST",
            &format!("{go_http_base}/search/do"),
            Some(json!({"keyword": SEARCH_FIXTURE_KEYWORD, "pageSize": 20})),
            "search-preflight",
        ),
    )
    .await
    {
        Ok(Ok(observation)) => observation,
        _ => return None,
    };
    if observation.status != 200
        || observation.body.get("status").and_then(Value::as_str) != Some("SUCCESS")
    {
        return None;
    }
    let data = match observation.body.get("data") {
        Some(data) => data,
        None => return None,
    };
    let all_pages_non_empty = ["users", "channels", "messageGroups"].iter().all(|key| {
        data.get(*key)
            .and_then(|page| page.get("items"))
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    });
    all_pages_non_empty
        .then(|| extract_search_fixture_seed(&observation.body))
        .flatten()
}

/// 自动从现有账号池找出同一 companyId 下的 actor/observer/third 三个身份。
pub(crate) async fn discover_auth_context() -> Result<AuthContext, String> {
    discover_auth_context_at(go_http_base(), go_health_url()).await
}

/// 从指定 Go 端点验证 health 后寻找三个共享租户账号，避免跨 profile 误连。
pub(crate) async fn discover_auth_context_at(
    go_http_base: &str,
    go_health_url: &str,
) -> Result<AuthContext, String> {
    discover_auth_context_at_mode(go_http_base, go_health_url, false).await
}

/// Search 专用的账号发现必须先通过真实 `search/do`，禁止回退到未验证身份。
async fn discover_search_auth_context_at(
    go_http_base: &str,
    go_health_url: &str,
) -> Result<AuthContext, String> {
    discover_auth_context_at_mode(go_http_base, go_health_url, true).await
}

/// 发现共享租户账号；`require_search` 只约束 Search Gate，避免 ES 波动影响其他 Gate。
async fn discover_auth_context_at_mode(
    go_http_base: &str,
    go_health_url: &str,
    require_search: bool,
) -> Result<AuthContext, String> {
    let health = anonymous_http_at(go_http_base, go_health_base_to_ws(go_health_url)).await?;
    let health_response = request_json(&health, "GET", go_health_url, None, "health").await?;
    if health_response.status != 200 {
        return Err(format!(
            "Go health {go_health_url} returned HTTP {}",
            health_response.status
        ));
    }
    if is_local_messagev3_profile() {
        return Ok(local_chain_auth_context());
    }

    let content = load_credentials_content()?;
    let credentials: CredentialFile = serde_json::from_str(&content)
        .map_err(|error| format!("parse credentials file failed: {error}"))?;
    let accounts = credentials.accounts();
    if accounts.len() < 3 {
        return Err("credentials account pool must contain at least three accounts".to_string());
    }

    let mut order = Vec::with_capacity(accounts.len());
    order.push(credentials.default_index().min(accounts.len() - 1));
    let selected = order.clone();
    order.extend((0..accounts.len()).filter(|index| !selected.contains(index)));
    let mut logged_in = Vec::new();
    let mut fallback = None;
    for index in order {
        let role = match logged_in.len() {
            0 => "actor",
            1 => "observer",
            _ => "third",
        };
        if let Ok(persona) = login_account(&accounts[index], role).await {
            logged_in.push(persona);
        }
        if logged_in.len() >= 3 {
            let actor = &logged_in[0];
            for observer_index in 1..logged_in.len() {
                for third_index in 1..logged_in.len() {
                    if observer_index == third_index {
                        continue;
                    }
                    let Some(company_id) = actor.companies.iter().find(|company| {
                        logged_in[observer_index].companies.contains(company)
                            && logged_in[third_index].companies.contains(company)
                    }) else {
                        continue;
                    };
                    let mut actor = actor.clone();
                    let mut observer = logged_in[observer_index].clone();
                    let mut third = logged_in[third_index].clone();
                    actor.company_id = company_id.clone();
                    observer.company_id = company_id.clone();
                    third.company_id = company_id.clone();
                    if fallback.is_none() {
                        fallback = Some(AuthContext {
                            actor: actor.clone(),
                            observer: observer.clone(),
                            third: third.clone(),
                        });
                    }
                    if !require_search
                        || search_observer_preflight(&observer, go_http_base, go_health_url)
                            .await
                            .is_some()
                    {
                        return Ok(AuthContext {
                            actor,
                            observer,
                            third,
                        });
                    }
                }
            }
        }
    }
    if !require_search {
        return fallback.ok_or_else(|| {
            "could not find three logged-in accounts sharing one companyId".to_string()
        });
    }
    Err(format!(
        "no shared account trio passed the real Search preflight at {go_http_base}/search/do"
    ))
}

/// 构造 local profile 快照中的三用户身份；Cookieid 依照本地 C006 合同等于 userId。
fn local_chain_auth_context() -> AuthContext {
    let persona = |user_id: &str, user_name: &str| Persona {
        user_id: user_id.to_string(),
        cookie_id: user_id.to_string(),
        company_id: CHAIN_COMPANY_ID.to_string(),
        user_name: user_name.to_string(),
        org_name: "本地真实链".to_string(),
        dept_name: "本地真实链".to_string(),
        companies: vec![CHAIN_COMPANY_ID.to_string()],
    };
    AuthContext {
        actor: persona(CHAIN_ACTOR_USER_ID, "本地发送者"),
        observer: persona(CHAIN_OBSERVER_USER_ID, "本地观察者"),
        third: persona(CHAIN_THIRD_USER_ID, "本地成员"),
    }
}

/// 将本轮 health URL 的 HTTP 端口映射为 NativeHttp 所需的 WS URL。
fn go_health_base_to_ws(health_url: &str) -> &str {
    if health_url == go_health_url() {
        go_ws_url()
    } else {
        chain_go_ws_url()
    }
}

/// 构造带 Go 身份头的 native HTTP client；HTTP actor 与 body userId 必须保持一致。
pub(crate) async fn go_http(persona: &Persona) -> Result<NativeHttp, String> {
    go_http_at(persona, go_http_base(), go_ws_url()).await
}

/// 构造绑定指定 Go 端点的身份 HTTP client，供 Phase5 使用。
pub(crate) async fn go_http_at(
    persona: &Persona,
    http_base: &str,
    ws_url: &str,
) -> Result<NativeHttp, String> {
    go_http_at_with_metrics(persona, http_base, ws_url, Arc::new(NoopMetricSink)).await
}

/// Engine 的 HTTP adapter 与其他端口共用同一 sink；辅助请求保持原有 Noop 边界。
async fn go_http_at_with_metrics(
    persona: &Persona,
    http_base: &str,
    ws_url: &str,
    sink: Arc<dyn AsyncMetricSink>,
) -> Result<NativeHttp, String> {
    let http =
        NativeHttp::with_config_and_metric_sink(HostNetworkConfig::new(http_base, ws_url), sink)
            .map_err(|error| format!("Go NativeHttp init failed: {error}"))?;
    for (name, value) in [
        ("Cookieid", persona.cookie_id.as_str()),
        ("userId", persona.user_id.as_str()),
        ("companyId", persona.company_id.as_str()),
        ("teamId", persona.company_id.as_str()),
        ("deviceId", "helix-phase1-rust"),
        ("appType", "bct"),
        ("device", "IOS"),
        ("language", "zh"),
        ("appVersion", "1.0.120"),
        ("timeZone", "Asia/Shanghai"),
        ("osVersion", "16.5.1"),
        ("siteName", "b"),
        ("envType", "pre"),
    ] {
        http.set_global_header(name, value)
            .await
            .map_err(|error| format!("set Go header {name}: {error}"))?;
    }
    Ok(http)
}

/// 生成 WS upgrade 所需身份头；不放入密码、cookie 内容或长期令牌。
pub(crate) fn ws_headers(persona: &Persona) -> Vec<(String, String)> {
    vec![
        ("Cookieid".to_string(), persona.cookie_id.clone()),
        ("userId".to_string(), persona.user_id.clone()),
        ("companyId".to_string(), persona.company_id.clone()),
        ("teamId".to_string(), persona.company_id.clone()),
        ("deviceId".to_string(), "helix-phase1-rust".to_string()),
    ]
}

/// 打开真实 Go WS，先完成 upgrade/activation，再由调用方消费 Connected 与 hello。
pub(crate) async fn open_raw_ws(persona: &Persona, raw_id: u32) -> Result<RawWs, String> {
    open_raw_ws_at(persona, raw_id, go_ws_url()).await
}

/// 打开指定 Go 端点的原始 WS，用于证明真实 fan-out frame 而不是本地 EventSink。
pub(crate) async fn open_raw_ws_at(
    persona: &Persona,
    raw_id: u32,
    ws_url: &str,
) -> Result<RawWs, String> {
    let (tick_tx, ticks) = mpsc::channel(256);
    let mut transport = NativeTransport::new(ws_url, TransportId::from_raw(raw_id), Some(tick_tx))
        .with_handshake_headers(ws_headers(persona));
    // 将 WS 外部等待限定在测试预算内，避免半关闭的 Go socket 令整批 Gate 永久悬挂。
    let activation = match timeout(Duration::from_secs(30), transport.connect()).await {
        Ok(Ok(activation)) => activation,
        Ok(Err(error)) => {
            let close_result = close_send_transport(Arc::new(transport), "real Go WS").await;
            return Err(match close_result {
                Ok(()) => format!("real Go WS connect failed: {error}"),
                Err(close_error) => format!("real Go WS connect failed: {error}; {close_error}"),
            });
        }
        Err(_) => {
            let close_result = close_send_transport(Arc::new(transport), "real Go WS").await;
            return Err(match close_result {
                Ok(()) => "real Go WS connect timed out after 30s".to_string(),
                Err(close_error) => {
                    format!("real Go WS connect timed out after 30s; {close_error}")
                }
            });
        }
    };
    let transport = Arc::new(transport);
    match timeout(Duration::from_secs(30), activation.activate()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            let close_result = close_send_transport(Arc::clone(&transport), "real Go WS").await;
            return Err(match close_result {
                Ok(()) => format!("real Go WS activation failed: {error}"),
                Err(close_error) => {
                    format!("real Go WS activation failed: {error}; {close_error}")
                }
            });
        }
        Err(_) => {
            let close_result = close_send_transport(Arc::clone(&transport), "real Go WS").await;
            return Err(match close_result {
                Ok(()) => "real Go WS activation timed out after 30s".to_string(),
                Err(close_error) => {
                    format!("real Go WS activation timed out after 30s; {close_error}")
                }
            });
        }
    }
    Ok(RawWs {
        transport,
        ticks,
        pending: VecDeque::new(),
    })
}

/// 等待指定 action 的真实 Go WS frame；乱序业务帧会留在 RawWs 缓存中，不被静默丢弃。
pub(crate) async fn wait_ws_action<F>(
    ws: &mut RawWs,
    action: &str,
    predicate: F,
    timeout_duration: Duration,
) -> Result<Value, String>
where
    F: Fn(&Value) -> bool,
{
    timeout(timeout_duration, async {
        loop {
            if let Some(frame) = ws.take_pending(&[action], &predicate) {
                return Ok(frame);
            }
            let frame = ws.receive_frame().await?;
            if frame.get("action").and_then(Value::as_str) == Some(action) && predicate(&frame) {
                return Ok(frame);
            }
            ws.remember(frame);
        }
    })
    .await
    .map_err(|_| format!("timed out waiting real Go WS action '{action}'"))?
}

/// 等待多个合法 action 中的第一个匹配帧，适用于 Go 对 urgent 同时广播 update_channel/post_update。
pub(crate) async fn wait_ws_actions<F>(
    ws: &mut RawWs,
    actions: &[&str],
    predicate: F,
    timeout_duration: Duration,
) -> Result<Value, String>
where
    F: Fn(&Value) -> bool,
{
    timeout(timeout_duration, async {
        loop {
            if let Some(frame) = ws.take_pending(actions, &predicate) {
                return Ok(frame);
            }
            let frame = ws.receive_frame().await?;
            let action = frame.get("action").and_then(Value::as_str);
            if actions.iter().any(|candidate| Some(*candidate) == action) && predicate(&frame) {
                return Ok(frame);
            }
            ws.remember(frame);
        }
    })
    .await
    .map_err(|_| format!("timed out waiting real Go WS actions {actions:?}"))?
}

/// 只等待一次 hello，作为 WS authenticated session 的业务前置证据。
pub(crate) async fn wait_ws_hello(ws: &mut RawWs) -> Result<Value, String> {
    wait_ws_action(ws, "hello", |_| true, Duration::from_secs(10)).await
}

/// 精确等待领域事件；超时仅保留最近16条事件身份元数据，不记录业务载荷。
pub(crate) async fn wait_domain_event<F>(
    receiver: &mut broadcast::Receiver<DomainEventBytes>,
    event_name: &str,
    predicate: F,
    timeout_duration: Duration,
) -> Result<DomainEventBytes, String>
where
    F: Fn(&Value) -> bool,
{
    let mut observed = std::collections::VecDeque::with_capacity(16);
    timeout(timeout_duration, async {
        loop {
            let event = receiver
                .recv()
                .await
                .map_err(|error| format!("event sink receive error: {error}"))?;
            let envelope: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("invalid Helix DomainEvent JSON: {error}"))?;
            if observed.len() == 16 {
                observed.pop_front();
            }
            observed.push_back(json!({
                "event": envelope.get("event"),
                "id": envelope.pointer("/data/id"),
                "temporaryId": envelope.pointer("/data/temporaryId"),
                "channelId": envelope.pointer("/data/channelId"),
                "eventSeq": envelope.pointer("/data/eventSeq"),
            }));
            if envelope.get("event").and_then(Value::as_str) == Some(event_name)
                && predicate(&envelope)
            {
                return Ok(event);
            }
        }
    })
    .await
    .map_err(|_| {
        format!("timed out waiting Helix event '{event_name}'; recent_event_metadata={observed:?}")
    })?
}

/// 从 event envelope 的 data 中取一个字符串字段，兼容 snake/camel wire。
pub(crate) fn data_string(envelope: &Value, keys: &[&str]) -> Option<String> {
    let data = envelope.get("data")?;
    string_field(data, keys)
}

/// 判断 terminal DomainEvent 是否明确属于当前 run-owned channel，拒绝无关事件误匹配。
fn event_targets_channel(envelope: &Value, channel_id: &str) -> bool {
    data_string(envelope, &["channelId", "channel_id", "id"]).as_deref() == Some(channel_id)
        || envelope
            .pointer("/data/channel")
            .and_then(|channel| string_field(channel, &["id", "channelId", "channel_id"]))
            .as_deref()
            == Some(channel_id)
}

/// 从 Go post/channel 响应中提取 canonical id，拒绝用 message 或随机字段冒充 id。
pub(crate) fn extract_id(value: &Value, keys: &[&str]) -> Option<String> {
    if let Some(id) = string_field(value, keys) {
        return Some(id);
    }
    match value {
        Value::Object(object) => object.values().find_map(|child| extract_id(child, keys)),
        Value::Array(items) => items.iter().find_map(|child| extract_id(child, keys)),
        _ => None,
    }
}

/// 将远端 JSON 数字或兼容历史字符串字段解析为毫秒整数。
fn json_i64(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number.as_i64(),
        Some(Value::String(text)) => text.parse::<i64>().ok(),
        _ => None,
    }
}

/// 返回当前 Unix 毫秒，供 run-owned 频道的新鲜度和快照有效期校验使用。
fn unix_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

/// 拒绝旧的、关闭的或身份字段不匹配的 channel/create/read-back 结果。
fn validate_active_channel_projection(
    channel: &Value,
    expected_id: &str,
    expected_display_name: &str,
    now_ms: i64,
) -> Result<(), String> {
    let actual_id = string_field(channel, &["id", "channelId", "channel_id"])
        .ok_or_else(|| "channel projection has no canonical id".to_string())?;
    if actual_id != expected_id {
        return Err(format!(
            "channel projection id mismatch: expected {expected_id}, got {actual_id}"
        ));
    }
    let display_name = string_field(channel, &["displayName", "display_name"])
        .ok_or_else(|| "channel projection has no displayName".to_string())?;
    if display_name != expected_display_name {
        return Err(format!(
            "channel projection displayName mismatch: expected {expected_display_name}, got {display_name}"
        ));
    }
    if let Some(channel_type) = string_field(channel, &["type"]) {
        if channel_type != "P" {
            return Err(format!(
                "run-owned fixture must be private group type P, got {channel_type}"
            ));
        }
    }
    if let Some(delete_at) = json_i64(channel.get("deleteAt").or_else(|| channel.get("delete_at")))
    {
        if delete_at != 0 {
            return Err(format!(
                "channel projection is expired: deleteAt={delete_at}"
            ));
        }
    }
    let closed = match channel.get("close").or_else(|| channel.get("closed")) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    };
    if closed {
        return Err("channel projection is closed".to_string());
    }
    let created_at = json_i64(channel.get("createAt").or_else(|| channel.get("create_at")))
        .ok_or_else(|| "channel projection has no createAt".to_string())?;
    if created_at < now_ms.saturating_sub(CHANNEL_FIXTURE_FRESHNESS_MS)
        || created_at > now_ms.saturating_add(CHANNEL_FIXTURE_FRESHNESS_MS)
    {
        return Err(format!(
            "channel projection is not run-owned fresh: createAt={created_at}, now={now_ms}"
        ));
    }
    Ok(())
}

/// 选择最新且仍开放的成员快照，并要求本轮三 persona 全部存在。
fn validate_active_member_snapshot(
    response: &Value,
    channel_id: &str,
    required_user_ids: &BTreeSet<String>,
    now_ms: i64,
) -> Result<(), String> {
    let data = response
        .pointer("/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "channel member snapshot response has no data array".to_string())?;
    let snapshot = data
        .iter()
        .filter(|candidate| {
            string_field(candidate, &["channelId", "channel_id"]).as_deref() == Some(channel_id)
        })
        .max_by_key(|candidate| json_i64(candidate.get("startTime")).unwrap_or_default())
        .ok_or_else(|| format!("no member snapshot for channel {channel_id}"))?;
    let snapshot_id = string_field(snapshot, &["snapshotId", "snapshot_id"])
        .ok_or_else(|| "member snapshot has no snapshotId".to_string())?;
    if snapshot_id.is_empty() {
        return Err("member snapshot has an empty snapshotId".to_string());
    }
    if let Some(end_time) = json_i64(snapshot.get("endTime").or_else(|| snapshot.get("end_time"))) {
        if end_time > 0 && end_time <= now_ms {
            return Err(format!(
                "member snapshot is expired: snapshotId={snapshot_id}, endTime={end_time}"
            ));
        }
    }
    let members = snapshot
        .get("members")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("member snapshot {snapshot_id} has no members array"))?;
    let actual_user_ids = members
        .iter()
        .filter_map(|member| string_field(member, &["userId", "user_id"]))
        .collect::<BTreeSet<_>>();
    let missing = required_user_ids
        .difference(&actual_user_ids)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "member snapshot {snapshot_id} missing members: {missing:?}"
        ));
    }
    Ok(())
}

/// 在 channel/create 后等待精确频道 read-back 与 active member snapshot 同时稳定。
async fn wait_active_channel_fixture(
    http: &NativeHttp,
    http_base: &str,
    channel_id: &str,
    display_name: &str,
    required_user_ids: &BTreeSet<String>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(CHANNEL_FIXTURE_WAIT_SECS);
    let mut last_error = String::new();
    while Instant::now() < deadline {
        let channel_result = request_json(
            http,
            "POST",
            &format!("{http_base}/channel/query"),
            Some(json!({
                "id": channel_id,
                "pageNumber": 1,
                "pageSize": 1,
                "offset": 0
            })),
            "channel-fixture-readback",
        )
        .await
        .and_then(|observation| require_success(observation, "channel/query"));
        let channel_check = channel_result.and_then(|response| {
            let data = response
                .pointer("/data")
                .ok_or_else(|| "channel/query SUCCESS response has no data".to_string())?;
            let row = match data {
                Value::Array(rows) => rows
                    .iter()
                    .find(|row| {
                        string_field(row, &["id", "channelId", "channel_id"]).as_deref()
                            == Some(channel_id)
                    })
                    .ok_or_else(|| {
                        format!("channel/query did not return run-owned channel {channel_id}")
                    })?,
                Value::Object(_) => data,
                _ => return Err("channel/query data is neither array nor object".to_string()),
            };
            validate_active_channel_projection(row, channel_id, display_name, unix_now_ms())
        });
        if let Err(error) = channel_check {
            last_error = error;
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }

        let snapshot_check = request_json(
            http,
            "POST",
            &format!("{http_base}/channel/member/snapshot"),
            Some(json!({
                "channelId": channel_id,
                "startTime": 0,
                "endTime": CHANNEL_FIXTURE_SNAPSHOT_END_MS
            })),
            "channel-fixture-snapshot",
        )
        .await
        .and_then(|observation| require_success(observation, "channel/member/snapshot"))
        .and_then(|response| {
            validate_active_member_snapshot(&response, channel_id, required_user_ids, unix_now_ms())
        });
        match snapshot_check {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(format!(
        "run-owned channel fixture did not become active within {CHANNEL_FIXTURE_WAIT_SECS}s: {last_error}"
    ))
}

/// 在业务证据链结束后先关闭 Helix session，再只清理本轮 run-owned 频道。
async fn finish_run_owned_channel<T>(
    context: &AuthContext,
    channel_id: &str,
    result: Result<T, String>,
) -> Result<T, String> {
    let cleanup = close_channel(context, channel_id).await;
    match (result, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!("channel cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => {
            Err(format!("{error}; channel cleanup failed: {cleanup_error}"))
        }
    }
}

/// 以 canonical channel/create body 创建 run-owned 私有群聊。
pub(crate) async fn create_channel(context: &AuthContext) -> Result<String, String> {
    if phase_pool_enabled() {
        phase_pool_channel_at(context, go_http_base(), go_ws_url()).await
    } else {
        create_channel_at(context, go_http_base(), go_ws_url()).await
    }
}

/// 通过公开 `im_create_channel` Command 完成真实建群链，并逐跳核对 Go/WS/SQLite/EventSink。
pub(crate) async fn assert_channel_create_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let request_id = run_token("channel-create");
    let display_name = resource_name("channel-command");
    let payload = json!({
        "type": "P",
        "user_ids": [context.observer.user_id.clone(), context.third.user_id.clone()],
        "display_name": display_name.clone(),
        "force_create": true,
        "req_id": request_id.clone(),
    });
    let mut session = match start_real_helix_session_as(&context, &context.actor, &[]).await {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Helix channel-create session could not be started",
                    "first_failing_layer": "Helix native transport/auth",
                    "command": "im_create_channel",
                    "go_http": "/api/cses/channel/create",
                    "error": error,
                    "corr_key": request_id,
                }),
            )?;
            return Ok(());
        }
    };

    let mut channel_id_for_cleanup = None::<String>;
    let chain_result = async {
        session.command("im_create_channel", payload.clone()).await?;
        let channel_frame = session
            .wait_ws_action(
                "channel_created",
                |frame| {
                    let Some(data) = frame.get("data") else {
                        return false;
                    };
                    string_field(data, &["displayName", "display_name"]).as_deref()
                        == Some(display_name.as_str())
                        && string_field(data, &["id", "channelId", "channel_id"]).is_some()
                },
                Duration::from_secs(30),
            )
            .await?;
        let authority = channel_frame
            .get("data")
            .ok_or_else(|| format!("{gate} channel_created frame has no data: {channel_frame}"))?;
        let channel_id = string_field(authority, &["id", "channelId", "channel_id"])
            .ok_or_else(|| format!("{gate} channel_created frame has no canonical id: {channel_frame}"))?;
        if helix_im::state::ChannelId::from_str(&channel_id).is_none() {
            return Err(format!("{gate} channel_created id is not canonical: {channel_id}"));
        }
        channel_id_for_cleanup = Some(channel_id.clone());
        let created = session
            .wait_event(
                "im:channel:created",
                |envelope| {
                    data_string(envelope, &["channelId", "channel_id", "id"]).as_deref()
                        == Some(channel_id.as_str())
                },
                Duration::from_secs(30),
            )
            .await?;
        let created_json: Value = serde_json::from_slice(created.0.as_ref())
            .map_err(|error| format!("{gate} channel-created event JSON failed: {error}"))?;
        if data_string(&created_json, &["displayName", "display_name"]).as_deref()
            != Some(display_name.as_str())
            || created_json.pointer("/data/tracing/requestId").and_then(Value::as_str)
                != Some(request_id.as_str())
        {
            return Err(format!(
                "{gate} channel-created projection fields mismatch: {created_json}"
            ));
        }

        let sqlite_channel = scan_channel_projection(&session.storage, &channel_id).await?;
        if row_text(&sqlite_channel, "id").as_deref() != Some(channel_id.as_str())
            || row_text(&sqlite_channel, "display_name").as_deref() != Some(display_name.as_str())
            || row_text(&sqlite_channel, "type").as_deref() != Some("P")
        {
            return Err(format!(
                "{gate} SQLite channel projection mismatch: {sqlite_channel:?}"
            ));
        }
        let member_rows = session
            .storage
            .scan(ScanSpec {
                table: "channel_member",
                limit: Some(16),
                filter: Some(("channel_id", SqlValue::Text(channel_id.clone()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("{gate} SQLite channel_member readback failed: {error}"))?;
        let actual_members = member_rows
            .iter()
            .filter_map(|row| row_text(row, "user_id"))
            .collect::<BTreeSet<_>>();
        let expected_members = BTreeSet::from([
            context.actor.user_id.clone(),
            context.observer.user_id.clone(),
            context.third.user_id.clone(),
        ]);
        if !expected_members.is_subset(&actual_members) {
            return Err(format!(
                "{gate} SQLite channel_member readback missing personas: expected={expected_members:?}, actual={actual_members:?}"
            ));
        }

        // Command 的 HTTP response 不在 NativeHttp port 暴露；用同一频道的真实 query 成功回读证明 Go
        // authority 已落远端，再由 raw channel_created frame 证明 WS authority。
        let http = go_http(&context.actor).await?;
        let query = request_json(
            &http,
            "POST",
            &format!("{}/channel/query", go_http_base()),
            Some(json!({"id": channel_id, "pageNumber": 1, "pageSize": 1, "offset": 0})),
            "channel-create-readback",
        )
        .await?;
        let query_body = require_success(query, "channel/query")?;
        if !query_body.to_string().contains(display_name.as_str())
            || !query_body.to_string().contains(channel_id.as_str())
        {
            return Err(format!("{gate} Go channel/query omitted created channel: {query_body}"));
        }
        let snapshot = request_json(
            &http,
            "POST",
            &format!("{}/channel/member/snapshot", go_http_base()),
            Some(json!({"channelId": channel_id, "startTime": 0, "endTime": CHANNEL_FIXTURE_SNAPSHOT_END_MS})),
            "channel-create-snapshot",
        )
        .await
        .and_then(|observation| require_success(observation, "channel/member/snapshot"))?;
        if !expected_members
            .iter()
            .all(|user_id| snapshot.to_string().contains(user_id))
        {
            return Err(format!("{gate} Go member snapshot omitted a created member: {snapshot}"));
        }
        Ok::<_, String>(json!({
            "channel_id": channel_id,
            "request_id": request_id,
            "display_name": display_name,
            "ws_frame": channel_frame,
            "event": created_json,
            "sqlite_channel": {"id": row_text(&sqlite_channel, "id"), "display_name": row_text(&sqlite_channel, "display_name"), "type": row_text(&sqlite_channel, "type")},
            "sqlite_members": actual_members,
            "go_channel_query": query_body,
            "go_member_snapshot": snapshot,
        }))
    }
    .await;
    let session_cleanup = session.close().await;
    let result = match (chain_result, session_cleanup) {
        (Ok(details), Ok(())) => Ok(details),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(format!(
            "{gate} Helix channel-create session cleanup failed: {error}"
        )),
        (Err(error), Err(cleanup_error)) => {
            Err(format!("{error}; session cleanup failed: {cleanup_error}"))
        }
    };
    let remote_cleanup = match channel_id_for_cleanup.as_deref() {
        Some(channel_id) => close_channel(&context, channel_id).await,
        None => Ok(()),
    };
    match (result, remote_cleanup) {
        (Ok(details), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "PASS",
                json!({
                    "command": "im_create_channel",
                    "go_http": ["/api/cses/channel/create", "/api/cses/channel/query", "/api/cses/channel/member/snapshot"],
                    "http_business": "Go channel/query and member/snapshot returned SUCCESS for the Command-created channel",
                    "ws_action": "channel_created",
                    "projection": "im:channel:created",
                    "persist_after": "channel + channel_member rows read back after PersistOk",
                    "event_sink": "NativeEventSink canonical DomainEvent",
                    "event_sink_required": true,
                    "sqlite_required": true,
                    "sqlite_readback": {"tables": ["channel", "channel_member"], "channel_id": details["channel_id"]},
                    "cleanup": {"status": "PASS", "remote_channel": details["channel_id"], "session": "closed"},
                    "corr_key": request_id,
                    "details": details,
                }),
            )?;
            Ok(())
        }
        (Err(error), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "NOT_VERIFIED",
                json!({
                    "reason": "channel-create Command did not reach a complete real-chain proof",
                    "command": "im_create_channel",
                    "go_http": "/api/cses/channel/create",
                    "corr_key": request_id,
                    "channel_id": channel_id_for_cleanup,
                    "error": error,
                }),
            )?;
            Ok(())
        }
        (Ok(_), Err(error)) => Err(format!("{gate} remote channel cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; remote channel cleanup failed: {cleanup_error}"
        )),
    }
}

/// 在真实 NativeStorage 上回放草稿 save/query 的公开 Tick 链，验证本地持久化与结果总线。
pub(crate) async fn assert_local_draft_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (storage, db_path) = open_test_storage().await?;
    let (event_sink, mut event_rx) = NativeEventSink::new();
    let mut config = ImConfig::default();
    config.auth_user_id = CHAIN_ACTOR_USER_ID.to_string();
    config.company_id = CHAIN_COMPANY_ID.to_string();
    let mut module = ImModule::new(config);
    let channel_id = "chfixx00000000000000000001".to_string();
    let text = resource_name("draft");
    let save_req = run_token("draft-save");
    let save_payload = serde_json::to_vec(&json!({
        "channel_id": channel_id.clone(),
        "text": text.clone(),
        "props": {"source": "messagev3"},
        "req_id": save_req.clone(),
    }))
    .map_err(|error| format!("{gate} draft save payload failed: {error}"))?;
    let mut sink = helix_core::EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new("im_save_draft", Bytes::from(save_payload))),
            1_800_000_000_000,
            &mut sink,
        )
        .map_err(|error| format!("{gate} im_save_draft Tick failed: {error}"))?;
    let (save_corr, save_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .ok_or_else(|| format!("{gate} im_save_draft did not emit Persist"))?;
    if sink.as_slice().iter().any(|effect| {
        matches!(
            effect,
            Effect::Http { .. } | Effect::Send { .. } | Effect::ScheduleTimer { .. }
        )
    }) {
        return Err(format!(
            "{gate} draft save emitted a forbidden external effect"
        ));
    }
    let save_outcome = execute_storage_ops(&storage, &save_ops).await;
    module
        .handle(
            &Tick::PortReply {
                corr: save_corr,
                outcome: save_outcome,
            },
            1_800_000_000_001,
            &mut sink,
        )
        .map_err(|error| format!("{gate} im_save_draft PersistOk failed: {error}"))?;
    let save_event = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => Some(event.clone()),
            _ => None,
        })
        .ok_or_else(|| format!("{gate} im_save_draft PersistOk emitted no result"))?;
    event_sink.emit(save_event.clone());
    let save_json: Value = serde_json::from_slice(save_event.0.as_ref())
        .map_err(|error| format!("{gate} draft save result JSON failed: {error}"))?;
    if save_json.get("event").and_then(Value::as_str) != Some("im:read:result")
        || save_json
            .pointer("/data/body/draft/text")
            .and_then(Value::as_str)
            != Some(text.as_str())
    {
        return Err(format!("{gate} draft save result mismatch: {save_json}"));
    }
    let _save_sink_event = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .map_err(|_| format!("{gate} NativeEventSink did not receive draft save result"))?
        .map_err(|error| format!("{gate} draft save EventSink closed: {error}"))?;
    let stored = storage
        .scoped_get(ScopedGetSpec {
            table: "message_draft",
            scope_col: "account_id",
            scope_val: SqlValue::Text(CHAIN_ACTOR_USER_ID.to_string()),
            key_col: "channel_id",
            key_val: SqlValue::Text(channel_id.clone()),
        })
        .await
        .map_err(|error| format!("{gate} draft SQLite readback failed: {error}"))?
        .ok_or_else(|| format!("{gate} draft row missing after PersistOk"))?;
    if row_text(&stored, "text").as_deref() != Some(text.as_str())
        || row_text(&stored, "account_id").as_deref() != Some(CHAIN_ACTOR_USER_ID)
    {
        return Err(format!("{gate} draft SQLite row mismatch: {stored:?}"));
    }

    sink.clear();
    let query_req = run_token("draft-query");
    let query_payload = serde_json::to_vec(&json!({
        "channel_id": channel_id.clone(),
        "req_id": query_req.clone(),
    }))
    .map_err(|error| format!("{gate} draft query payload failed: {error}"))?;
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_query_draft",
                Bytes::from(query_payload),
            )),
            1_800_000_000_002,
            &mut sink,
        )
        .map_err(|error| format!("{gate} im_query_draft Tick failed: {error}"))?;
    let (query_corr, query_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .ok_or_else(|| format!("{gate} im_query_draft did not emit ScopedGet"))?;
    let query_outcome = execute_storage_ops(&storage, &query_ops).await;
    module
        .handle(
            &Tick::PortReply {
                corr: query_corr,
                outcome: query_outcome,
            },
            1_800_000_000_003,
            &mut sink,
        )
        .map_err(|error| format!("{gate} im_query_draft readback failed: {error}"))?;
    let query_event = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => Some(event.clone()),
            _ => None,
        })
        .ok_or_else(|| format!("{gate} im_query_draft emitted no result"))?;
    event_sink.emit(query_event.clone());
    let query_json: Value = serde_json::from_slice(query_event.0.as_ref())
        .map_err(|error| format!("{gate} draft query result JSON failed: {error}"))?;
    if query_json.get("event").and_then(Value::as_str) != Some("im:read:result")
        || query_json
            .pointer("/data/body/draft/text")
            .and_then(Value::as_str)
            != Some(text.as_str())
    {
        return Err(format!("{gate} draft query result mismatch: {query_json}"));
    }
    let _query_sink_event = timeout(Duration::from_secs(2), event_rx.recv())
        .await
        .map_err(|_| format!("{gate} NativeEventSink did not receive draft query result"))?
        .map_err(|error| format!("{gate} draft query EventSink closed: {error}"))?;

    drop(module);
    drop(storage);
    remove_test_storage(&db_path);
    if db_path.exists()
        || PathBuf::from(format!("{}-wal", db_path.display())).exists()
        || PathBuf::from(format!("{}-shm", db_path.display())).exists()
    {
        return Err(format!("{gate} draft SQLite cleanup left a sidecar"));
    }
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": ["im_save_draft", "im_query_draft"],
            "go_http": "N/A: local HostStorage contract",
            "http_business": "N/A: draft is explicitly local and emits no remote HTTP",
            "projection": "im:read:result command result",
            "persist_after": "message_draft row read back after PersistOk",
            "event_sink": "NativeEventSink:im:read:result",
            "event_sink_required": true,
            "sqlite_required": true,
            "sqlite_readback": {"table": "message_draft", "account_id": CHAIN_ACTOR_USER_ID, "channel_id": channel_id, "text": text},
            "cleanup": {"status": "PASS", "sqlite": "db-wal-shm removed"},
            "corr_key": query_req,
            "save_req_id": save_req,
        }),
    )?;
    Ok(())
}

/// 预热当前 Phase 的最多五个 run-owned 频道，并返回可循环复用的频道 ID 列表。
async fn phase_pool_channels_at(
    context: &AuthContext,
    http_base: &str,
    ws_url: &str,
) -> Result<Vec<String>, String> {
    let pool = phase_resource_pool().await;
    let mut state = pool.lock().await;
    if let Some(existing) = state.http_base.as_deref() {
        if existing != http_base || state.ws_url.as_deref() != Some(ws_url) {
            return Err(format!(
                "Phase resource pool endpoint changed from {existing}/{} to {http_base}/{ws_url}",
                state.ws_url.as_deref().unwrap_or("<missing>")
            ));
        }
    } else {
        state.http_base = Some(http_base.to_string());
        state.ws_url = Some(ws_url.to_string());
    }
    while state
        .channels
        .iter()
        .filter(|channel_id| !state.retired_channels.contains(*channel_id))
        .count()
        < PHASE_POOL_MAX_CHANNELS
    {
        match create_channel_at(context, http_base, ws_url).await {
            Ok(channel_id) => state.channels.push(channel_id),
            Err(error) => {
                let created = std::mem::take(&mut state.channels);
                drop(state);
                let mut cleanup_errors = Vec::new();
                for channel_id in created {
                    if let Err(cleanup_error) =
                        close_channel_at(context, &channel_id, http_base, ws_url).await
                    {
                        cleanup_errors.push(cleanup_error);
                    }
                }
                return if cleanup_errors.is_empty() {
                    Err(error)
                } else {
                    Err(format!(
                        "{error}; phase channel prewarm cleanup failed: {}",
                        cleanup_errors.join("; ")
                    ))
                };
            }
        }
    }
    Ok(state
        .channels
        .iter()
        .filter(|channel_id| !state.retired_channels.contains(*channel_id))
        .cloned()
        .collect())
}

/// 取得一个 Phase 频道槽位；测试内串行执行时按确定性 round-robin 复用。
async fn phase_pool_channel_at(
    context: &AuthContext,
    http_base: &str,
    ws_url: &str,
) -> Result<String, String> {
    phase_pool_channels_at(context, http_base, ws_url).await?;
    let pool = phase_resource_pool().await;
    let mut state = pool.lock().await;
    let active = state
        .channels
        .iter()
        .filter(|channel_id| !state.retired_channels.contains(*channel_id))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return Err("Phase resource pool has no active channel slots".to_string());
    }
    let index = state.next_channel % active.len();
    let selected = active[index].clone();
    state.next_channel = state.next_channel.saturating_add(1);
    Ok(selected)
}

/// 将会破坏频道后续可复用性的 mutation 标记为 retired，下一次租赁时补一个活跃槽位。
pub(crate) async fn retire_phase_pool_channel(channel_id: &str) {
    if !phase_pool_enabled() {
        return;
    }
    let pool = phase_resource_pool().await;
    let mut state = pool.lock().await;
    if state
        .channels
        .iter()
        .any(|candidate| candidate == channel_id)
    {
        state.retired_channels.insert(channel_id.to_string());
    }
}

/// 判断频道是否由当前 Phase 池持有，避免 Gate 结束时提前调用远端 channel/close。
async fn is_phase_pool_channel(channel_id: &str) -> bool {
    if !phase_pool_enabled() {
        return false;
    }
    phase_resource_pool()
        .await
        .lock()
        .await
        .channels
        .iter()
        .any(|candidate| candidate == channel_id)
}

/// 在 Phase 专属 cleanup test 中停止共享 session/WS 并关闭最多五个远端频道。
pub(crate) async fn shutdown_messagev3_phase_pool(phase: u8) -> Result<(), String> {
    if !phase_pool_enabled() {
        return Ok(());
    }
    let pool = phase_resource_pool().await;
    let (channels, sessions, http_base, ws_url) = {
        let mut state = pool.lock().await;
        (
            std::mem::take(&mut state.channels),
            std::mem::take(&mut state.sessions),
            state.http_base.take(),
            state.ws_url.take(),
        )
    };
    if channels.is_empty() && sessions.is_empty() {
        return Ok(());
    }
    let context = if phase == 5 {
        CHAIN_AUTH_CONTEXT
            .get_or_init(|| async {
                discover_auth_context_at(chain_go_http_base(), go_health_url())
                    .await
                    .map(Arc::new)
            })
            .await
            .clone()
            .map_err(|error| error.to_string())?
    } else {
        AUTH_CONTEXT
            .get_or_init(|| async { discover_auth_context().await.map(Arc::new) })
            .await
            .clone()
            .map_err(|error| error.to_string())?
    };
    let mut errors = Vec::new();
    let resources = sessions.into_values().collect::<Vec<_>>();
    let db_paths = resources
        .iter()
        .map(|resource| resource.db_path.clone())
        .collect::<Vec<_>>();
    for resource in resources {
        if let Err(error) = shutdown_pooled_resource_on_phase_runtime(resource).await {
            errors.push(error);
        }
    }
    let http_base = http_base.unwrap_or_else(|| go_http_base().to_string());
    let ws_url = ws_url.unwrap_or_else(|| go_ws_url().to_string());
    for channel_id in channels {
        if let Err(error) = close_channel_at(&context, &channel_id, &http_base, &ws_url).await {
            errors.push(format!(
                "phase channel {channel_id} cleanup failed: {error}"
            ));
        }
    }
    // 释放最后一个 session Arc 后再删除 SQLite，保证 WAL/SHM 不会被仍存活的连接重建。
    for db_path in db_paths {
        remove_test_storage(&db_path);
        if db_path.exists()
            || PathBuf::from(format!("{}-wal", db_path.display())).exists()
            || PathBuf::from(format!("{}-shm", db_path.display())).exists()
        {
            errors.push(format!(
                "phase pooled SQLite cleanup left files at {}",
                db_path.display()
            ));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// 在指定 Go 端点创建三成员 run-owned 频道，供 Phase5 使用。
pub(crate) async fn create_chain_channel(context: &AuthContext) -> Result<String, String> {
    if phase_pool_enabled() {
        phase_pool_channel_at(context, chain_go_http_base(), chain_go_ws_url()).await
    } else {
        create_channel_at(context, chain_go_http_base(), chain_go_ws_url()).await
    }
}

/// 创建并验证指定端点上的 run-owned 频道及成员快照。
async fn create_channel_at(
    context: &AuthContext,
    http_base: &str,
    ws_url: &str,
) -> Result<String, String> {
    let http = go_http_at(&context.actor, http_base, ws_url).await?;
    let display_name = resource_name("channel");
    let started_at = unix_now_ms();
    let body = json!({
        "type": "P",
        "userIds": [context.observer.user_id, context.third.user_id],
        "displayName": display_name,
        "forceCreate": true
    });
    let observation = request_json(
        &http,
        "POST",
        &format!("{http_base}/channel/create"),
        Some(body),
        "channel-create",
    )
    .await?;
    let response = require_success(observation, "channel/create")?;
    let channel = response
        .pointer("/data")
        .filter(|value| value.is_object())
        .ok_or_else(|| "channel/create SUCCESS response has no channel object".to_string())?;
    let channel_id = string_field(channel, &["id"])
        .ok_or_else(|| "channel/create SUCCESS response has no canonical channel id".to_string())?;
    if let Err(error) =
        validate_active_channel_projection(channel, &channel_id, &display_name, started_at)
    {
        let cleanup = close_channel_at(context, &channel_id, http_base, ws_url).await;
        return Err(match cleanup {
            Ok(()) => format!("channel/create projection validation failed: {error}"),
            Err(cleanup_error) => {
                format!("channel/create projection validation failed: {error}; channel cleanup failed: {cleanup_error}")
            }
        });
    }
    let required_user_ids = BTreeSet::from([
        context.actor.user_id.clone(),
        context.observer.user_id.clone(),
        context.third.user_id.clone(),
    ]);
    if let Err(error) = wait_active_channel_fixture(
        &http,
        http_base,
        &channel_id,
        &display_name,
        &required_user_ids,
    )
    .await
    {
        let cleanup = close_channel_at(context, &channel_id, http_base, ws_url).await;
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup_error) => format!("{error}; channel cleanup failed: {cleanup_error}"),
        });
    }
    Ok(channel_id)
}

/// 只关闭本测试刚创建的频道；Phase 池中的频道延迟到批次末统一回收。
pub(crate) async fn close_channel(context: &AuthContext, channel_id: &str) -> Result<(), String> {
    if is_phase_pool_channel(channel_id).await {
        return Ok(());
    }
    close_channel_at(context, channel_id, go_http_base(), go_ws_url()).await
}

/// 关闭 Phase5 本轮 profile 上创建的 run-owned 频道；池内资源由 Phase cleanup 处理。
pub(crate) async fn close_chain_channel(
    context: &AuthContext,
    channel_id: &str,
) -> Result<(), String> {
    if is_phase_pool_channel(channel_id).await {
        return Ok(());
    }
    close_channel_at(context, channel_id, chain_go_http_base(), chain_go_ws_url()).await
}

/// 通过指定 Go 端点关闭单个本轮频道，拒绝全库清理。
async fn close_channel_at(
    context: &AuthContext,
    channel_id: &str,
    http_base: &str,
    ws_url: &str,
) -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..3 {
        let http = go_http_at(&context.actor, http_base, ws_url).await?;
        let result = request_json(
            &http,
            "POST",
            &format!("{http_base}/channel/close"),
            Some(json!({ "channelId": channel_id })),
            "channel-close",
        )
        .await
        .and_then(|observation| require_success(observation, "channel/close").map(|_| ()));
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < 2
                    && (error.contains("deadlock") || error.contains("serialization")) =>
            {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(250 * (attempt + 1) as u64)).await;
            }
            Err(error) => return Err(error),
        }
    }
    Err(last_error.unwrap_or_else(|| "channel cleanup exhausted retry budget".to_string()))
}

#[cfg(test)]
mod gate_observation_regression_tests {
    use super::*;

    /// 成功业务遇到清理失败仍失败，同时保留业务首因与两个清理错误。
    #[test]
    fn cleanup_failure_cannot_publish_pass() {
        assert!(finish_gate_cleanup(Ok(()), Err("session".into()), Ok(())).is_err());
        assert!(finish_gate_cleanup(Ok(()), Ok(()), Err("channel".into())).is_err());
        let error = finish_gate_cleanup::<()>(
            Err("business".into()),
            Err("session".into()),
            Err("channel".into()),
        )
        .unwrap_err();
        assert!(error.starts_with("business;"));
        assert!(error.contains("session cleanup failed"));
        assert!(error.contains("channel cleanup failed"));
    }

    /// 启动恢复突发不能挤掉先到的目标终态，单Gate与整Phase使用同一观察预算。
    #[tokio::test]
    async fn recovery_burst_keeps_the_earlier_terminal_event() {
        let (sink, mut receiver) = new_session_event_sink();
        let terminal = DomainEventBytes(Bytes::from_static(
            br#"{"event":"im:post:received","data":{"temporaryId":"target"}}"#,
        ));
        sink.emit(terminal.clone());
        for _ in 0..2048 {
            sink.emit(DomainEventBytes(Bytes::from_static(
                br#"{"event":"im:channel:update","data":{}}"#,
            )));
        }
        let observed = wait_domain_event(
            &mut receiver,
            "im:post:received",
            |value| value.pointer("/data/temporaryId").and_then(Value::as_str) == Some("target"),
            Duration::from_secs(1),
        )
        .await
        .expect("recovery burst must preserve terminal evidence");
        assert_eq!(observed.0, terminal.0);
    }

    /// 超预算必须显式失败，禁止跳过Lagged后把剩余终态当作完整证据。
    #[tokio::test]
    async fn overflow_still_fails_closed() {
        let (sink, mut receiver) = NativeEventSink::new_with_capacity(2);
        for _ in 0..4 {
            sink.emit(DomainEventBytes(Bytes::from_static(
                br#"{"event":"im:post:received","data":{}}"#,
            )));
        }
        let error = wait_domain_event(
            &mut receiver,
            "im:post:received",
            |_| true,
            Duration::from_secs(1),
        )
        .await
        .expect_err("lagged evidence must fail");
        assert!(error.contains("lagged"));
    }
}

#[cfg(test)]
mod channel_fixture_regression_tests {
    use super::*;

    #[test]
    fn stale_channel_readback_is_rejected_before_gate_seed() {
        let now_ms = 1_800_000_000_000_i64;
        let stale = json!({
            "id": "stale-channel",
            "displayName": "hx-p1-channel-old",
            "type": "P",
            "createAt": now_ms - CHANNEL_FIXTURE_FRESHNESS_MS - 1,
            "deleteAt": now_ms - 1,
            "close": true
        });

        let error = validate_active_channel_projection(
            &stale,
            "stale-channel",
            "hx-p1-channel-old",
            now_ms,
        )
        .expect_err("closed/expired channel must not be accepted as a run-owned fixture");
        assert!(error.contains("deleteAt") || error.contains("close"));
    }

    #[test]
    fn active_snapshot_requires_all_run_personas_and_open_end() {
        let now_ms = 1_800_000_000_000_i64;
        let required = BTreeSet::from([
            "actor".to_string(),
            "observer".to_string(),
            "third".to_string(),
        ]);
        let incomplete = json!({
            "data": [{
                "channelId": "run-channel",
                "snapshotId": "snapshot-1",
                "startTime": now_ms - 100,
                "endTime": 0,
                "members": [{"userId": "actor", "role": "CREATOR"}]
            }]
        });

        let error = validate_active_member_snapshot(&incomplete, "run-channel", &required, now_ms)
            .expect_err("a snapshot without the active observer/third members is not usable");
        assert!(error.contains("missing members"));
    }
}

#[cfg(test)]
mod credential_file_regression_tests {
    use super::*;

    /// 客户端当前的数组账号池格式必须能被 Native real-chain 直接读取。
    #[test]
    fn credential_pool_accepts_client_array_format() {
        let pool: CredentialFile =
            serde_json::from_str(r#"[{"mobile":"13800000000","password":"redacted"}]"#)
                .expect("array credential pool should deserialize");
        assert_eq!(pool.accounts().len(), 1);
        assert_eq!(pool.default_index(), 0);
    }

    /// 兼容旧的对象账号池格式及其 defaultIndex 选择规则。
    #[test]
    fn credential_pool_accepts_object_format() {
        let pool: CredentialFile = serde_json::from_str(
            r#"{"accounts":[{"mobile":"13800000001","password":"redacted"}],"defaultIndex":3}"#,
        )
        .expect("object credential pool should deserialize");
        assert_eq!(pool.accounts().len(), 1);
        assert_eq!(pool.default_index(), 3);
    }

    /// 内联账号池必须保持原始 JSON，供统一解析器读取。
    #[test]
    fn inline_credential_pool_accepts_non_empty_json() {
        let content = r#"[{"mobile":"13800000002","password":"redacted"}]"#.to_string();
        assert_eq!(
            validate_inline_credentials(Some(content.clone())).expect("inline JSON should pass"),
            Some(content)
        );
    }

    /// 显式配置的空账号池必须 fail-closed，不能意外读取开发机凭据。
    #[test]
    fn inline_credential_pool_rejects_empty_json() {
        let error = validate_inline_credentials(Some("  \n".to_string()))
            .expect_err("empty inline JSON must fail closed");
        assert!(error.contains("configured but empty"));
    }
}

/// 在真实 Go 上直接发送一条 post，返回 HTTP admission 和 observer 的 outbound post frame。
pub(crate) async fn direct_post(
    context: &AuthContext,
    channel_id: &str,
    message_type: &str,
    text: &str,
    props: Value,
    viewers: Value,
    mentions: Value,
) -> Result<(Value, Value), String> {
    direct_post_as(
        &context.actor,
        &context.observer,
        channel_id,
        message_type,
        text,
        props,
        viewers,
        mentions,
        10,
    )
    .await
}

/// 以指定 sender 创建 post，并由指定 watcher 观察真实 Go `post` WS 回声。
///
/// Phase2/Phase4 的双用户 Gate 使用此 helper 证明 reader/author audience，而不是只在
/// 同一个 actor session 内看到自己的 HTTP admission。
pub(crate) async fn direct_post_as(
    sender: &Persona,
    watcher: &Persona,
    channel_id: &str,
    message_type: &str,
    text: &str,
    props: Value,
    viewers: Value,
    mentions: Value,
    raw_id: u32,
) -> Result<(Value, Value), String> {
    if phase_pool_enabled() {
        return pooled_direct_post_as(
            sender,
            watcher,
            channel_id,
            message_type,
            text,
            props,
            viewers,
            mentions,
        )
        .await;
    }
    let mut observer = open_raw_ws(watcher, raw_id).await?;
    let result = async {
        wait_ws_hello(&mut observer).await?;
        let temporary_id = run_token("post");
        let http = go_http(sender).await?;
        let body = json!({
            "channelId": channel_id,
            // Angular buildMessageObject always carries the active company/team scope;
            // Go exact/timeline reads use this field to enforce tenant visibility.
            "teamId": sender.company_id,
            "userId": sender.user_id,
            "type": message_type,
            "message": text,
            "simpleMessage": text,
            "props": props,
            "viewers": viewers,
            "mentions": mentions,
            "temporaryId": temporary_id
        });
        let observation = request_json(
            &http,
            "POST",
            &format!("{}/posts/create", go_http_base()),
            Some(body),
            "post-create",
        )
        .await?;
        let response = require_success(observation, "posts/create")?;
        let post = wait_ws_action(
            &mut observer,
            "post",
            |frame| {
                extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await?;
        Ok::<(Value, Value), String>((response, post))
    }
    .await;
    let close_result = observer
        .transport
        .close()
        .await
        .map_err(|error| format!("close observer WS: {error}"));
    match (result, close_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(close_error)) => {
            Err(format!("{error}; close observer WS failed: {close_error}"))
        }
    }
}

/// 使用 Phase 账号池的 watcher session 观察直接 Go post，避免每条 post 新建 raw WS。
async fn pooled_direct_post_as(
    sender: &Persona,
    watcher: &Persona,
    channel_id: &str,
    message_type: &str,
    text: &str,
    props: Value,
    viewers: Value,
    mentions: Value,
) -> Result<(Value, Value), String> {
    let context = AUTH_CONTEXT
        .get()
        .ok_or_else(|| "Phase pooled direct post requires initialized AuthContext".to_string())?
        .clone()
        .map_err(|error| error.to_string())?;
    let mut watcher_session = start_real_helix_session_as(&context, watcher, &[channel_id]).await?;
    let temporary_id = run_token("post");
    let http = go_http(sender).await?;
    let body = json!({
        "channelId": channel_id,
        "teamId": sender.company_id,
        "userId": sender.user_id,
        "type": message_type,
        "message": text,
        "simpleMessage": text,
        "props": props,
        "viewers": viewers,
        "mentions": mentions,
        "temporaryId": temporary_id
    });
    let result = async {
        let observation = request_json(
            &http,
            "POST",
            &format!("{}/posts/create", go_http_base()),
            Some(body),
            "post-create",
        )
        .await?;
        let response = require_success(observation, "posts/create")?;
        let post = watcher_session
            .wait_ws_action(
                "post",
                |frame| {
                    extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(20),
            )
            .await?;
        Ok::<(Value, Value), String>((response, post))
    }
    .await;
    let close_result = watcher_session.close().await;
    match (result, close_result) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(close_error)) => Err(format!(
            "{error}; close pooled watcher failed: {close_error}"
        )),
    }
}

/// 初始化真实 IM SQLite schema；只对本测试新建数据库执行 CREATE/migration。
pub(crate) async fn apply_im_schema(storage: &NativeStorage) -> Result<(), String> {
    for statement in IM_SCHEMA {
        storage
            .execute_raw(statement)
            .await
            .map_err(|error| format!("IM_SCHEMA failed for {statement:?}: {error}"))?;
    }
    for statement in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(statement).await {
            if !error.to_string().contains("duplicate column name") {
                return Err(format!(
                    "IM_SCHEMA_MIGRATIONS failed for {statement:?}: {error}"
                ));
            }
        }
    }
    Ok(())
}

/// 打开本地 run-owned SQLite 文件；不会接触 cses-client 的共享 tenant.db。
pub(crate) async fn open_test_storage() -> Result<(NativeStorage, PathBuf), String> {
    let path = std::env::temp_dir().join(format!("{}.db", resource_name("sqlite")));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let storage = match NativeStorage::open(&url).await {
        Ok(storage) => storage,
        Err(error) => {
            remove_test_storage(&path);
            return Err(format!("open test SQLite failed: {error}"));
        }
    };
    if let Err(error) = apply_im_schema(&storage).await {
        drop(storage);
        remove_test_storage(&path);
        return Err(error);
    }
    Ok((storage, path))
}

/// 删除本测试自己的 SQLite 主文件与 sidecar，不触碰共享客户端数据库。
pub(crate) fn remove_test_storage(path: &Path) {
    let _ = std::fs::remove_file(path);
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.display(), suffix));
        let _ = std::fs::remove_file(sidecar);
    }
}

#[cfg(test)]
mod send_cleanup_regression_tests {
    use super::*;

    /// 即使 engine 尚未启动，清理上下文也必须释放本轮 SQLite 文件。
    #[tokio::test]
    async fn cleanup_removes_owned_sqlite_without_started_engine() {
        let (storage, db_path) = open_test_storage()
            .await
            .expect("test storage should open for cleanup regression");
        let cleanup = SendCleanup::new(storage, db_path.clone());
        cleanup
            .close()
            .await
            .expect("cleanup without engine or transport should succeed");
        assert!(!db_path.exists());
        assert!(!PathBuf::from(format!("{}-wal", db_path.display())).exists());
        assert!(!PathBuf::from(format!("{}-shm", db_path.display())).exists());
    }

    /// 满观察队列也能关闭并回收reader，不能等待无人消费的Disconnected。
    #[tokio::test]
    async fn observer_close_releases_full_tick_queue() {
        use futures_util::StreamExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(message) = socket.next().await {
                if matches!(
                    message,
                    Ok(tokio_tungstenite::tungstenite::Message::Close(_))
                ) {
                    break;
                }
            }
        });
        let (tick_tx, ticks) = mpsc::channel(1);
        let mut transport = NativeTransport::new(&url, TransportId::from_raw(901), Some(tick_tx));
        let activation = transport.connect().await.unwrap();
        activation.activate().await.unwrap();
        assert_eq!(ticks.len(), 1, "Connected must fill the queue before close");
        let observer = RawWs {
            transport: Arc::new(transport),
            ticks,
            pending: VecDeque::new(),
        };
        timeout(Duration::from_secs(2), observer.close())
            .await
            .expect("close must not wait for the abandoned observer")
            .unwrap();
        timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    /// observer 收到迟到业务帧时只保留有界缓存，避免 cleanup 期间无界积压。
    #[test]
    fn observer_late_frames_are_bounded() {
        let (tick_tx, ticks) = mpsc::channel(1);
        let transport = NativeTransport::new(
            "ws://127.0.0.1:0/never",
            TransportId::from_raw(900),
            Some(tick_tx),
        );
        let mut observer = RawWs {
            transport: Arc::new(transport),
            ticks,
            pending: VecDeque::new(),
        };
        for sequence in 0..=MAX_PENDING_WS_FRAMES {
            observer.remember(json!({"action": "post", "sequence": sequence}));
        }
        assert_eq!(observer.pending.len(), MAX_PENDING_WS_FRAMES);
        assert_eq!(
            observer
                .pending
                .front()
                .and_then(|frame| frame["sequence"].as_u64()),
            Some(1)
        );
    }
}

/// 扫描一个 temporary_id 的 durable message row，作为 Persist/WS 后的行级 readback。
pub(crate) async fn scan_message_row(
    storage: &NativeStorage,
    temporary_id: &str,
) -> Result<Row, String> {
    let rows = storage
        .scan(ScanSpec {
            table: "message",
            limit: Some(4),
            filter: Some(("temporary_id", SqlValue::Text(temporary_id.to_string()))),
            order_by: &[],
        })
        .await
        .map_err(|error| format!("scan message row failed: {error}"))?;
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("message row {temporary_id} not found after PersistOk"))
}

/// 扫描当前 mutation 对应的 channel projection，作为 Native Store 持久化读回锚点。
async fn scan_channel_projection(storage: &NativeStorage, channel_id: &str) -> Result<Row, String> {
    let rows = storage
        .scan(ScanSpec {
            table: "channel",
            limit: Some(4),
            filter: Some(("id", SqlValue::Text(channel_id.to_string()))),
            order_by: &[],
        })
        .await
        .map_err(|error| format!("scan channel projection failed: {error}"))?;
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("channel projection {channel_id} not found after PersistOk"))
}

/// 按服务端 `id` 或 exact-read 归一后的 `temporary_id` 读取一条消息行。
///
/// 公告保存由 Go 重新分配 post id，而 Helix exact-post cache 可能保留两种键；
/// 这里仅在同一条消息的两个 canonical 键中查找，不用模糊频道扫描冒充持久化证据。
pub(crate) async fn scan_message_row_by_id_or_temporary(
    storage: &NativeStorage,
    message_id: &str,
) -> Result<Row, String> {
    for (column, label) in [("id", "id"), ("temporary_id", "temporary_id")] {
        let rows = storage
            .scan(ScanSpec {
                table: "message",
                limit: Some(4),
                filter: Some((column, SqlValue::Text(message_id.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("scan message row by {label} failed: {error}"))?;
        if let Some(row) = rows.into_iter().next() {
            return Ok(row);
        }
    }
    Err(format!(
        "message row {message_id} not found by id or temporary_id after PersistOk"
    ))
}

/// 等待真实 nickname PersistFire 写入当前 run 的 channel_member 复合键行。
pub(crate) async fn wait_channel_member_nickname(
    storage: &NativeStorage,
    channel_id: &str,
    user_id: &str,
    expected_nick_name: &str,
    wait_for: Duration,
) -> Result<Row, String> {
    let deadline = Instant::now() + wait_for;
    loop {
        let rows = storage
            .scan(ScanSpec {
                table: "channel_member",
                limit: Some(128),
                filter: Some(("channel_id", SqlValue::Text(channel_id.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("scan channel_member nickname row failed: {error}"))?;
        if let Some(row) = rows.into_iter().find(|row| {
            row_text(row, "user_id").as_deref() == Some(user_id)
                && row_text(row, "nick_name").as_deref() == Some(expected_nick_name)
        }) {
            return Ok(row);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "channel_member ({channel_id}, {user_id}) did not reach nick_name={expected_nick_name:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 等待 notify-only WS 写入 channel 与 channel_member 后的 SQLite 双层读回。
pub(crate) async fn wait_channel_notify(
    storage: &NativeStorage,
    channel_id: &str,
    user_id: &str,
    expected_notify: &str,
    wait_for: Duration,
) -> Result<(Row, Row), String> {
    let deadline = Instant::now() + wait_for;
    loop {
        let channel_rows = storage
            .scan(ScanSpec {
                table: "channel",
                limit: Some(8),
                filter: Some(("id", SqlValue::Text(channel_id.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("scan channel notify row failed: {error}"))?;
        let member_rows = storage
            .scan(ScanSpec {
                table: "channel_member",
                limit: Some(8),
                filter: Some(("channel_id", SqlValue::Text(channel_id.to_string()))),
                order_by: &[],
            })
            .await
            .map_err(|error| format!("scan channel_member notify row failed: {error}"))?;
        let channel_snapshot = channel_rows.clone();
        let member_snapshot = member_rows.clone();
        if let (Some(channel), Some(member)) = (
            channel_rows
                .into_iter()
                .find(|row| row_text(row, "notify_props").as_deref() == Some(expected_notify)),
            member_rows.into_iter().find(|row| {
                row_text(row, "user_id").as_deref() == Some(user_id)
                    && row_text(row, "notify").as_deref() == Some(expected_notify)
            }),
        ) {
            return Ok((channel, member));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "channel/member notify rows did not reach {expected_notify:?}; channel_rows={channel_snapshot:?}; member_rows={member_snapshot:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 在真实 Persist/WS 链异步收敛期间轮询同一行，避免把合法的 sending 中间态误报为 terminal 失败。
pub(crate) async fn wait_message_status(
    storage: &NativeStorage,
    temporary_id: &str,
    expected: &[&str],
    wait_for: Duration,
) -> Result<Row, String> {
    let deadline = Instant::now() + wait_for;
    let mut last_status = None;
    loop {
        if let Ok(row) = scan_message_row(storage, temporary_id).await {
            let status = row_text(&row, "send_status");
            if status
                .as_deref()
                .map(|value| expected.iter().any(|candidate| candidate == &value))
                .unwrap_or(false)
            {
                return Ok(row);
            }
            last_status = status;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "message {temporary_id} did not reach {:?}; last status={last_status:?}",
                expected
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 等待同一 post 的 type6 authority 经过 PersistOk 落入本地 read_bits，避免 WS/Emit 先于
/// SQLite writer 完成时把合法的短暂空串误报为 Gate 失败。
async fn wait_message_read_bits(
    storage: &NativeStorage,
    post_id: &str,
    expected_bits: &str,
    wait_for: Duration,
) -> Result<Row, String> {
    let deadline = Instant::now() + wait_for;
    let mut last_row = None;
    loop {
        if let Ok(row) = scan_message_row_by_id_or_temporary(storage, post_id).await {
            let read_bits = row_text(&row, "read_bits");
            if read_bits.as_deref() == Some(expected_bits) {
                return Ok(row);
            }
            last_row = Some(row);
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "message {post_id} did not persist read_bits={expected_bits:?}; last row={last_row:?}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 读取 row 的文本列，避免把 Debug 输出当成 storage 证据。
pub(crate) fn row_text(row: &Row, column: &str) -> Option<String> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Text(text)) if name == column => Some(text.clone()),
            _ => None,
        })
}

/// 读取整型投影列，供 Chain revision/seq/count 的行级断言使用。
pub(crate) fn row_i64(row: &Row, column: &str) -> Option<i64> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Integer(number)) if name == column => Some(*number),
            _ => None,
        })
}

/// 按唯一键读取文字接龙 durable projection，避免用全表扫描掩盖持久化缺失。
pub(crate) async fn scan_chain_row(
    storage: &NativeStorage,
    table: &'static str,
    key_column: &'static str,
    key: &str,
) -> Result<Row, String> {
    let rows = storage
        .scan(ScanSpec {
            table,
            limit: Some(4),
            filter: Some((key_column, SqlValue::Text(key.to_string()))),
            order_by: &[],
        })
        .await
        .map_err(|error| format!("scan {table}.{key_column} failed: {error}"))?;
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("{table} row {key_column}={key} not found after PersistOk"))
}

/// 读取同一文字接龙的全部 Entry，按 durable seq 排序验证无重复/无洞。
pub(crate) async fn scan_chain_entries(
    storage: &NativeStorage,
    chain_id: &str,
) -> Result<Vec<Row>, String> {
    storage
        .scan(ScanSpec {
            table: "chain_entry",
            limit: Some(256),
            filter: Some(("chain_id", SqlValue::Text(chain_id.to_string()))),
            order_by: CHAIN_ENTRY_ORDER,
        })
        .await
        .map_err(|error| format!("scan chain_entry for {chain_id} failed: {error}"))
}

/// 按真实 engine 装配一次完整 send，返回 HTTP→WS→SQLite→emit 的逐跳证据。
pub(crate) async fn run_helix_send(
    context: &AuthContext,
    channel_id: &str,
    intent: SendIntent,
    send_timeout_ms: u64,
) -> Result<SendEvidence, String> {
    if phase_pool_enabled() {
        return run_pooled_helix_send(context, channel_id, intent).await;
    }
    let expected_type = canonical_wire_message_type(intent.message_type);
    let expected_text = intent.text.clone();
    let metric_runtime = metrics::start();
    let metric_sink = metric_runtime.sink();
    let (storage, db_path) = open_test_storage().await?;
    let mut cleanup = SendCleanup::new(storage.with_metric_sink(Arc::clone(&metric_sink)), db_path);
    let observer = match open_raw_ws(&context.observer, 20).await {
        Ok(observer) => observer,
        Err(error) => return finish_send_cleanup(Err(error), cleanup).await,
    };
    cleanup.observer = Some(observer);
    let (event_sink, mut events) = NativeEventSink::new();
    let event_sink = event_sink.with_metric_sink(Arc::clone(&metric_sink));
    let result = async {
        wait_ws_hello(cleanup.observer_mut()?).await?;
        let http = go_http_at_with_metrics(
            &context.actor,
            go_http_base(),
            go_ws_url(),
            Arc::clone(&metric_sink),
        )
        .await?;

        let mut config = ImConfig::default();
        config.ws_url = go_ws_url().to_string();
        config.api_base_url = go_http_base().to_string();
        config.default_api_base_url = main_api_base();
        config.auth_user_id = context.actor.user_id.clone();
        config.company_id = context.actor.company_id.clone();
        config.user_name = context.actor.user_name.clone();
        config.org_name = context.actor.org_name.clone();
        config.dept_name = context.actor.dept_name.clone();
        config.ping_interval_ms = messagev3_ping_interval_ms();
        config.send_timeout_ms = send_timeout_ms;

        let mut module = ImModule::new(config);
        let channel = helix_im::state::ChannelId::from_str(channel_id).ok_or_else(|| {
            format!("Go channel id is not a canonical Helix ChannelId: {channel_id}")
        })?;
        module.register_channel(channel, 0);
        let mut shell = ExecutionShell::new();
        shell.register(module);

        let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
        let mut actor_transport = NativeTransport::new(
            go_ws_url(),
            TransportId::from_raw(21),
            Some(tick_tx.clone()),
        )
        .with_handshake_headers(ws_headers(&context.actor))
        .with_metric_sink(Arc::clone(&metric_sink));
        let activation = match timeout(Duration::from_secs(30), actor_transport.connect()).await {
            Ok(Ok(activation)) => activation,
            Ok(Err(error)) => {
                let close_result =
                    close_send_transport(Arc::new(actor_transport), "Helix actor WS").await;
                return Err(match close_result {
                    Ok(()) => format!("Helix actor WS connect failed: {error}"),
                    Err(close_error) => {
                        format!("Helix actor WS connect failed: {error}; {close_error}")
                    }
                });
            }
            Err(_) => {
                let close_result =
                    close_send_transport(Arc::new(actor_transport), "Helix actor WS").await;
                return Err(match close_result {
                    Ok(()) => "Helix actor WS connect timed out after 30s".to_string(),
                    Err(close_error) => {
                        format!("Helix actor WS connect timed out after 30s; {close_error}")
                    }
                });
            }
        };
        let actor_transport = Arc::new(actor_transport);
        cleanup.actor_transport = Some(Arc::clone(&actor_transport));
        let mut transports = TransportTable::new();
        transports.insert(TransportId::from_raw(21), Arc::clone(&actor_transport));

        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let config = EngineConfig::new(
            cleanup.storage()?.clone(),
            http,
            SharedFileUploader::default(),
            event_sink,
            8,
        )
        .with_metric_sink(metric_sink);
        let engine_tick_tx = tick_tx.clone();
        let engine = tokio::spawn(async move {
            run_engine_loop_with_transports(
                shell,
                tick_rx,
                engine_tick_tx,
                config,
                shutdown_rx,
                transports,
            )
            .await;
            metrics::shutdown(metric_runtime).await;
        });
        cleanup.shutdown_tx = Some(shutdown_tx);
        cleanup.engine = Some(engine);
        timeout(Duration::from_secs(30), activation.activate())
            .await
            .map_err(|_| "Helix actor WS activation timed out after 30s".to_string())?
            .map_err(|error| format!("Helix actor WS activation failed: {error}"))?;

        wait_domain_event(
            &mut events,
            "im:connection:established",
            |_| true,
            Duration::from_secs(20),
        )
        .await?;

        let mut command = json!({
            "channel_id": channel_id,
            "type": intent.message_type,
            "text": intent.text,
            "props": intent.props,
            "viewers": intent.viewers,
            "mentions": intent.mentions
        });
        if let Some(replied) = intent.replied {
            command["replied"] = replied;
        }
        tick_tx
            .send(Tick::Command(AppCommand::new(
                "im_send_message",
                Bytes::from(serde_json::to_vec(&command).map_err(|error| error.to_string())?),
            )))
            .await
            .map_err(|error| format!("send im_send_message Tick failed: {error}"))?;

        let sending = wait_domain_event(
            &mut events,
            "im:post:sending",
            |_| true,
            Duration::from_secs(20),
        )
        .await?;
        let sending_value: Value = serde_json::from_slice(sending.0.as_ref())
            .map_err(|error| format!("decode sending event: {error}"))?;
        let temporary_id = data_string(&sending_value, &["temporaryId", "temporary_id"])
            .ok_or_else(|| "im:post:sending has no temporaryId".to_string())?;

        let optimistic_row = scan_message_row(cleanup.storage()?, &temporary_id).await?;
        if row_text(&optimistic_row, "send_status").as_deref() != Some("sending") {
            return Err(format!(
                "message {temporary_id} did not persist optimistic sending row: {:?}",
                row_text(&optimistic_row, "send_status")
            ));
        }

        let post_frame = wait_ws_action(
            cleanup.observer_mut()?,
            "post",
            |frame| {
                extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str())
            },
            Duration::from_secs(25),
        )
        .await?;
        for (field, expected) in [
            ("channelId", channel_id),
            ("userId", context.actor.user_id.as_str()),
            ("type", expected_type),
            ("message", expected_text.as_str()),
        ] {
            if extract_id(&post_frame, &[field]).as_deref() != Some(expected) {
                return Err(format!("observer WS post {field} mismatch: {post_frame}"));
            }
        }
        let received = wait_domain_event(
            &mut events,
            "im:post:received",
            |envelope| {
                data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str())
            },
            Duration::from_secs(25),
        )
        .await?;
        let row = wait_message_status(
            cleanup.storage()?,
            &temporary_id,
            &["sent"],
            Duration::from_secs(8),
        )
        .await?;
        for (column, expected) in [
            ("channel_id", channel_id),
            ("user_id", context.actor.user_id.as_str()),
            ("type", expected_type),
            ("message", expected_text.as_str()),
        ] {
            if row_text(&row, column).as_deref() != Some(expected) {
                return Err(format!(
                    "SQLite message.{column} mismatch for {temporary_id}"
                ));
            }
        }
        if row_text(&row, "id").is_none_or(|id| id.is_empty()) {
            return Err(format!("SQLite message.id missing for {temporary_id}"));
        }
        let bus_envelope = to_bus_envelope(&received);
        if bus_event_name(&received).as_deref() != Some("im:post:received") {
            return Err(
                "NativeEventSink event cannot map to im:post:received bus channel".to_string(),
            );
        }
        if bus_envelope.get("channel").and_then(Value::as_str) != Some("im:post:received") {
            return Err(format!("bad native bus envelope: {bus_envelope}"));
        }

        Ok::<SendEvidence, String>(SendEvidence {
            temporary_id,
            sending,
            post_frame,
            received,
            row,
            bus_envelope,
        })
    }
    .await;
    finish_send_cleanup(result, cleanup).await
}

/// 复用 Phase actor/watcher session 完成发送链；每个句柄仍拥有独立事件游标。
async fn run_pooled_helix_send(
    context: &AuthContext,
    channel_id: &str,
    intent: SendIntent,
) -> Result<SendEvidence, String> {
    let expected_type = canonical_wire_message_type(intent.message_type);
    let expected_text = intent.text.clone();
    // watcher 先上线，确保 direct Go post 的广播不会落在连接建立前。
    let mut watcher =
        start_real_helix_session_as(context, &context.observer, &[channel_id]).await?;
    let mut actor = match start_real_helix_session_as(context, &context.actor, &[channel_id]).await
    {
        Ok(session) => session,
        Err(error) => {
            let _ = watcher.close().await;
            return Err(error);
        }
    };
    let mut command = json!({
        "channel_id": channel_id,
        "type": intent.message_type,
        "text": intent.text,
        "props": intent.props,
        "viewers": intent.viewers,
        "mentions": intent.mentions
    });
    if let Some(replied) = intent.replied {
        command["replied"] = replied;
    }
    let result = async {
        actor.command("im_send_message", command).await?;
        let sending = actor
            .wait_event("im:post:sending", |_| true, Duration::from_secs(20))
            .await?;
        let sending_value: Value = serde_json::from_slice(sending.0.as_ref())
            .map_err(|error| format!("decode sending event: {error}"))?;
        let temporary_id = data_string(&sending_value, &["temporaryId", "temporary_id"])
            .ok_or_else(|| "im:post:sending has no temporaryId".to_string())?;
        let optimistic_row = scan_message_row(&actor.storage, &temporary_id).await?;
        if row_text(&optimistic_row, "send_status").as_deref() != Some("sending") {
            return Err(format!(
                "message {temporary_id} did not persist optimistic sending row: {:?}",
                row_text(&optimistic_row, "send_status")
            ));
        }
        let post_frame = watcher
            .wait_ws_action(
                "post",
                |frame| {
                    extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        for (field, expected) in [
            ("channelId", channel_id),
            ("userId", context.actor.user_id.as_str()),
            ("type", expected_type),
            ("message", expected_text.as_str()),
        ] {
            if extract_id(&post_frame, &[field]).as_deref() != Some(expected) {
                return Err(format!("observer WS post {field} mismatch: {post_frame}"));
            }
        }
        let received = match actor
            .wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await
        {
            Ok(received) => received,
            Err(error) => {
                // 失败诊断复用同一物理 WS tap，区分 sender 未收帧与 Helix gate 未释放终态。
                let actor_post = actor
                    .wait_ws_action(
                        "post",
                        |frame| {
                            extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                                == Some(temporary_id.as_str())
                        },
                        Duration::from_secs(1),
                    )
                    .await;
                let durable = scan_message_row(&actor.storage, &temporary_id).await;
                return Err(format!(
                    "{error}; actor_post={actor_post:?}; durable_row={durable:?}"
                ));
            }
        };
        let row = wait_message_status(
            &actor.storage,
            &temporary_id,
            &["sent"],
            Duration::from_secs(8),
        )
        .await?;
        for (column, expected) in [
            ("channel_id", channel_id),
            ("user_id", context.actor.user_id.as_str()),
            ("type", expected_type),
            ("message", expected_text.as_str()),
        ] {
            if row_text(&row, column).as_deref() != Some(expected) {
                return Err(format!(
                    "SQLite message.{column} mismatch for {temporary_id}"
                ));
            }
        }
        if row_text(&row, "id").is_none_or(|id| id.is_empty()) {
            return Err(format!("SQLite message.id missing for {temporary_id}"));
        }
        let bus_envelope = to_bus_envelope(&received);
        if bus_event_name(&received).as_deref() != Some("im:post:received") {
            return Err(
                "NativeEventSink event cannot map to im:post:received bus channel".to_string(),
            );
        }
        if bus_envelope.get("channel").and_then(Value::as_str) != Some("im:post:received") {
            return Err(format!("bad native bus envelope: {bus_envelope}"));
        }
        Ok::<SendEvidence, String>(SendEvidence {
            temporary_id,
            sending,
            post_frame,
            received,
            row,
            bus_envelope,
        })
    }
    .await;
    let actor_cleanup = actor.close().await;
    let watcher_cleanup = watcher.close().await;
    match (result, actor_cleanup, watcher_cleanup) {
        (Ok(value), Ok(()), Ok(())) => Ok(value),
        (Err(error), Ok(()), Ok(())) => Err(error),
        (Ok(_), actor, watcher) => {
            Err(format!("pooled send cleanup failed: {actor:?} {watcher:?}"))
        }
        (Err(error), actor, watcher) => Err(format!(
            "{error}; pooled send cleanup failed: {actor:?} {watcher:?}"
        )),
    }
}

/// 在 engine 上发送一个不存在的 canonical channel，验证 send-failed 而非假 received。
pub(crate) async fn run_helix_send_failure(
    context: &AuthContext,
    invalid_channel_id: &str,
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let metric_runtime = metrics::start();
    let metric_sink = metric_runtime.sink();
    let (storage, db_path) = open_test_storage().await?;
    let storage = storage.with_metric_sink(Arc::clone(&metric_sink));
    let mut cleanup = SendCleanup::new(storage.clone(), db_path);
    let result = async {
        let storage_for_engine = storage.clone();
        let (event_sink, mut events) = NativeEventSink::new();
        let event_sink = event_sink.with_metric_sink(Arc::clone(&metric_sink));
        let http = go_http_at_with_metrics(
            &context.actor,
            go_http_base(),
            go_ws_url(),
            Arc::clone(&metric_sink),
        )
        .await?;
        let mut config = ImConfig::default();
        config.ws_url = go_ws_url().to_string();
        config.api_base_url = go_http_base().to_string();
        config.default_api_base_url = main_api_base();
        config.auth_user_id = context.actor.user_id.clone();
        config.company_id = context.actor.company_id.clone();
        config.send_timeout_ms = 2_000;
        config.ping_interval_ms = messagev3_ping_interval_ms();
        let mut module = ImModule::new(config);
        module.register_channel(
            helix_im::state::ChannelId::from_str(invalid_channel_id)
                .ok_or_else(|| "failure test channel id is not canonical".to_string())?,
            0,
        );
        let mut shell = ExecutionShell::new();
        shell.register(module);
        let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
        let mut transport = NativeTransport::new(
            go_ws_url(),
            TransportId::from_raw(31),
            Some(tick_tx.clone()),
        )
        .with_handshake_headers(ws_headers(&context.actor))
        .with_metric_sink(Arc::clone(&metric_sink));
        let activation = transport
            .connect()
            .await
            .map_err(|error| format!("failure actor WS connect failed: {error}"))?;
        let transport = Arc::new(transport);
        cleanup.actor_transport = Some(Arc::clone(&transport));
        let mut transports = TransportTable::new();
        transports.insert(TransportId::from_raw(31), Arc::clone(&transport));
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let engine_tick_tx = tick_tx.clone();
        let engine = tokio::spawn(async move {
            run_engine_loop_with_transports(
                shell,
                tick_rx,
                engine_tick_tx,
                EngineConfig::new(
                    storage_for_engine,
                    http,
                    SharedFileUploader::default(),
                    event_sink,
                    8,
                )
                .with_metric_sink(metric_sink),
                shutdown_rx,
                transports,
            )
            .await;
            metrics::shutdown(metric_runtime).await;
        });
        cleanup.shutdown_tx = Some(shutdown_tx);
        cleanup.engine = Some(engine);
        activation
            .activate()
            .await
            .map_err(|error| format!("failure actor WS activation failed: {error}"))?;
        wait_domain_event(
            &mut events,
            "im:connection:established",
            |_| true,
            Duration::from_secs(20),
        )
        .await?;
        let command = json!({
            "channel_id": invalid_channel_id,
            "type": "TEXT",
            "text": "phase1 failure probe",
            "props": {},
            "viewers": ["all"],
            "mentions": []
        });
        tick_tx
            .send(Tick::Command(AppCommand::new(
                "im_send_message",
                Bytes::from(serde_json::to_vec(&command).map_err(|error| error.to_string())?),
            )))
            .await
            .map_err(|error| format!("failure command send failed: {error}"))?;
        let failed_event = wait_domain_event(
            &mut events,
            "im:post:send-failed",
            |_| true,
            Duration::from_secs(20),
        )
        .await?;
        let failed_value: Value = serde_json::from_slice(failed_event.0.as_ref())
            .map_err(|error| format!("decode send-failed event: {error}"))?;
        let failed_temporary_id = data_string(&failed_value, &["temporaryId", "temporary_id"])
            .ok_or_else(|| "im:post:send-failed has no temporaryId".to_string())?;
        let failed_row = wait_message_status(
            &storage,
            &failed_temporary_id,
            &["unsend", "failed"],
            Duration::from_secs(8),
        )
        .await?;
        let received_after_failure = wait_domain_event(
            &mut events,
            "im:post:received",
            |envelope| {
                data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(failed_temporary_id.as_str())
            },
            Duration::from_millis(500),
        )
        .await;
        if received_after_failure.is_ok() {
            return Err(format!(
                "failed message {failed_temporary_id} unexpectedly emitted im:post:received"
            ));
        }
        Ok((failed_temporary_id, failed_row, failed_value))
    }
    .await;
    let (failed_temporary_id, failed_row, failed_value) =
        finish_send_cleanup(result, cleanup).await?;
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": "im_send_message",
            "go_http": "/api/cses/posts/create",
            "http_business": "unknown channel was rejected; no authoritative post admission",
            "projection": "im:post:send-failed",
            "persist_after": "SQLite message row reached unsend/failed",
            "event_sink": "NativeEventSink:im:post:send-failed",
            "event_sink_required": true,
            "sqlite_required": true,
            "sqlite_readback": {
                "table": "message",
                "temporary_id": failed_temporary_id,
                "channel_id": invalid_channel_id,
                "send_status": row_text(&failed_row, "send_status"),
                "message": row_text(&failed_row, "message"),
            },
            "cleanup": {"status": "PASS", "transport": "closed", "sqlite": "removed"},
            "corr_key": failed_temporary_id,
            "received_absent": true,
            "failed_event": failed_value,
        }),
    )?;
    Ok(())
}

/// 断言直接 Go post 回声保留指定字段，不把 HTTP response 当 WS 证据。
pub(crate) fn assert_post_frame(
    frame: &Value,
    message_type: &str,
    text: &str,
) -> Result<(), String> {
    if string_field(frame, &["action"]).as_deref() != Some("post") {
        return Err(format!("expected action=post, got {frame}"));
    }
    let data = frame
        .get("data")
        .ok_or_else(|| "post frame missing data".to_string())?;
    if string_field(data, &["type"]).as_deref() != Some(message_type) {
        return Err(format!("post frame type mismatch: {frame}"));
    }
    if string_field(data, &["message"]).as_deref() != Some(text) {
        return Err(format!("post frame message mismatch: {frame}"));
    }
    Ok(())
}

/// 校验实际投影至少保留调用方提交的业务 props；Go 允许额外追加 event cursor 字段。
pub(crate) fn assert_props_subset(
    actual: &Value,
    expected: &Value,
    label: &str,
) -> Result<(), String> {
    let actual_object = actual
        .as_object()
        .ok_or_else(|| format!("{label} actual props is not an object"))?;
    let expected_object = expected
        .as_object()
        .ok_or_else(|| format!("{label} expected props is not an object"))?;
    for (key, expected_value) in expected_object {
        let actual_value = actual_object
            .get(key)
            .ok_or_else(|| format!("{label} missing props.{key}"))?;
        if expected_value.is_object() {
            assert_props_subset(actual_value, expected_value, &format!("{label}.{key}"))?;
        } else if actual_value != expected_value {
            return Err(format!(
                "{label} props.{key} mismatch: expected {expected_value}, got {actual_value}"
            ));
        }
    }
    Ok(())
}

/// 只核对媒体稳定引用和 Host 产出的公开元数据，忽略 Go/WS 追加的非业务字段。
pub(crate) fn assert_media_props(
    actual_props: &Value,
    expected_props: &Value,
    label: &str,
) -> Result<(), String> {
    let values_equal = |actual: Option<&Value>, expected: &Value| match (actual, expected) {
        (Some(Value::Number(actual)), Value::Number(expected)) => {
            actual.as_f64() == expected.as_f64()
        }
        (Some(actual), expected) => actual == expected,
        (None, _) => false,
    };
    let compare_node = |actual: &Value, expected: &Value, node_label: &str| {
        let actual_object = actual
            .as_object()
            .ok_or_else(|| format!("{node_label} actual media node is not object"))?;
        let expected_object = expected
            .as_object()
            .ok_or_else(|| format!("{node_label} expected media node is not object"))?;
        for key in ["bucket", "id", "uri", "contentType", "size", "sha256"] {
            let expected_value = expected_object
                .get(key)
                .ok_or_else(|| format!("{node_label} expected media node misses {key}"))?;
            let actual_value = actual_object.get(key).or_else(|| {
                (key == "name")
                    .then(|| actual_object.get("fileName"))
                    .flatten()
            });
            if !values_equal(actual_value, expected_value) {
                return Err(format!(
                    "{node_label} media {key} mismatch: expected {expected_value}, got {}",
                    actual_value.unwrap_or(&Value::Null)
                ));
            }
        }
        let expected_name = expected_object
            .get("name")
            .ok_or_else(|| format!("{node_label} expected media node misses name"))?;
        let actual_name = actual_object
            .get("name")
            .or_else(|| actual_object.get("fileName"));
        if !values_equal(actual_name, expected_name) {
            return Err(format!(
                "{node_label} media name mismatch: expected {expected_name}, got {}",
                actual_name.unwrap_or(&Value::Null)
            ));
        }
        if let Some(expected_duration) = expected_object.get("duration") {
            if !values_equal(actual_object.get("duration"), expected_duration) {
                return Err(format!(
                    "{node_label} media duration mismatch: expected {expected_duration}, got {}",
                    actual_object.get("duration").unwrap_or(&Value::Null)
                ));
            }
        }
        for key in ["width", "height"] {
            if let Some(expected_value) = expected_object.get(key) {
                if !values_equal(actual_object.get(key), expected_value) {
                    return Err(format!(
                        "{node_label} media {key} mismatch: expected {expected_value}, got {}",
                        actual_object.get(key).unwrap_or(&Value::Null)
                    ));
                }
            }
        }
        Ok::<(), String>(())
    };

    if let Some(expected_files) = expected_props.get("files").and_then(Value::as_array) {
        let actual_files = actual_props
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{label} received props.files is missing"))?;
        if actual_files.len() != expected_files.len() {
            return Err(format!(
                "{label} media file count mismatch: expected {}, got {}",
                expected_files.len(),
                actual_files.len()
            ));
        }
        for (index, (actual, expected)) in actual_files.iter().zip(expected_files).enumerate() {
            compare_node(actual, expected, &format!("{label}.files[{index}]"))?;
        }
        return Ok(());
    }

    if let Some(expected_file) = expected_props.get("file") {
        let actual_file = actual_props
            .get("file")
            .ok_or_else(|| format!("{label} received props.file is missing"))?;
        compare_node(actual_file, expected_file, &format!("{label}.file"))?;
        return Ok(());
    }

    if let Some(expected_template_file) = expected_props
        .pointer("/template/file")
        .or_else(|| expected_props.pointer("/template/files/0"))
    {
        let actual_template_file = actual_props
            .pointer("/template/file")
            .or_else(|| actual_props.pointer("/template/files/0"))
            .ok_or_else(|| format!("{label} received template media file is missing"))?;
        compare_node(
            actual_template_file,
            expected_template_file,
            &format!("{label}.template.file"),
        )?;
        return Ok(());
    }

    Err(format!(
        "{label} expected props has no supported media shape"
    ))
}

/// 从 Gate details 提取首个非空 correlation，缺失时显式返回 unbound。
fn evidence_corr_key(details: &Value) -> String {
    [
        "corr_key",
        "temporary_id",
        "req_id",
        "chain_id",
        "channel_id",
        "resource",
    ]
    .iter()
    .find_map(|key| details.get(key).and_then(Value::as_str))
    .filter(|value| !value.is_empty())
    .unwrap_or("unbound")
    .to_string()
}

/// 把测试类沿用的 PascalCase/斜杠 Domain 归一到 docs/mmd 的 canonical snake_case。
fn canonical_domain_name(domain: &str) -> String {
    match domain {
        "Chain/Common" => "chain".to_string(),
        "Chain/Text" => "text_chain".to_string(),
        "Chain/Category" => "category_chain".to_string(),
        "Chain/Acceptance" => "acceptance".to_string(),
        _ => domain
            .replace('/', "_")
            .chars()
            .enumerate()
            .flat_map(|(index, character)| {
                if index > 0 && character.is_ascii_uppercase() {
                    vec!['_', character.to_ascii_lowercase()]
                } else {
                    vec![character.to_ascii_lowercase()]
                }
            })
            .collect(),
    }
}

/// 为单个 Gate 写入当前 suite 的无 secret 机器证据，稳定路径禁止借用旧运行。
pub(crate) fn write_gate_evidence(
    phase: &str,
    domain: &str,
    gate: &str,
    status: &str,
    details: Value,
) -> Result<PathBuf, String> {
    let root = std::env::var("HELIX_MESSAGEV3_ARTIFACT_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../artifacts/messagev3-phase1-5-real-chain")
        });
    let suite_run_id = std::env::var("MESSAGEV3_SUITE_RUN_ID")
        .unwrap_or_else(|_| format!("standalone-{}", std::process::id()));
    let run_id = std::env::var("MESSAGEV3_GATE_RUN_ID")
        .unwrap_or_else(|_| format!("{}-{}", suite_run_id, gate));
    let canonical_domain = canonical_domain_name(domain);
    let canonical_gate = gate.to_ascii_uppercase();
    let path = root
        .join(&suite_run_id)
        .join(phase)
        .join(&canonical_domain)
        .join(&canonical_gate);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("create evidence directory failed: {error}"))?;
    let file = path.join("evidence.json");
    let mut details = details;
    annotate_phase_pool_evidence(&mut details);
    let corr_key = evidence_corr_key(&details);
    let payload = json!({
        "schema_version": "messagev3-gate-evidence/v2",
        "suite_run_id": suite_run_id,
        "run_id": run_id,
        "corr_key": corr_key,
        "phase": phase,
        "domain": canonical_domain,
        "gate": canonical_gate,
        "status": status,
        "details": details,
    });
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("serialize evidence failed: {error}"))?;
    std::fs::write(&file, bytes).map_err(|error| format!("write evidence failed: {error}"))?;
    Ok(file)
}

/// 给批量证据标出“逻辑租约结束、物理资源 Phase 末回收”，避免把复用误报成逐 Gate 断连。
fn annotate_phase_pool_evidence(details: &mut Value) {
    if !phase_pool_enabled() {
        return;
    }
    let Some(object) = details.as_object_mut() else {
        return;
    };
    object.insert(
        "phase_resource_pool".to_string(),
        json!({
            "enabled": true,
            "max_channels": PHASE_POOL_MAX_CHANNELS,
            "event_buffer_capacity": PHASE_POOL_EVENT_CAPACITY,
            "ws": "per-account session reused until phase cleanup",
            "logical_release": "gate end",
            "physical_cleanup": "phase cleanup test"
        }),
    );
    if let Some(cleanup) = object.get_mut("cleanup").and_then(Value::as_object_mut) {
        cleanup.insert(
            "transport".to_string(),
            Value::String("reused_until_phase_cleanup".to_string()),
        );
        cleanup.insert(
            "phase_pool".to_string(),
            Value::String("physical WS/SQLite/channel cleanup is phase-scoped".to_string()),
        );
    }
}

/// 运行标准文本/模板/文档发送链并断言 Persist、Go WS、terminal emit 与 SQLite。
pub(crate) async fn assert_send_chain(
    context: &AuthContext,
    channel_id: &str,
    intent: SendIntent,
) -> Result<SendEvidence, String> {
    let evidence = run_helix_send(context, channel_id, intent, 15_000).await?;
    let received: Value = serde_json::from_slice(evidence.received.0.as_ref())
        .map_err(|error| format!("decode received event: {error}"))?;
    if received.get("event").and_then(Value::as_str) != Some("im:post:received") {
        return Err("terminal event is not im:post:received".to_string());
    }
    if row_text(&evidence.row, "send_status").as_deref() != Some("sent") {
        return Err("durable terminal row is not sent".to_string());
    }
    if evidence.bus_envelope.get("channel").and_then(Value::as_str) != Some("im:post:received") {
        return Err("NativeEventSink bus channel is not im:post:received".to_string());
    }
    Ok(evidence)
}

/// 创建隔离频道、执行一个 Gate 真实发送场景并在结束后只清理本轮资源。
pub(crate) async fn assert_isolated_send(
    phase: &str,
    domain: &str,
    gate: &str,
    message_type: &'static str,
    text: String,
    props: Value,
    viewers: Vec<String>,
    mentions: Vec<String>,
    replied: Option<Value>,
) -> Result<SendEvidence, String> {
    let (_guard, context) = remote_setup().await;
    let channel_id = create_channel(&context).await?;
    let expected_props = props.clone();
    let result = assert_send_chain(
        &context,
        &channel_id,
        SendIntent {
            message_type,
            text,
            props,
            viewers,
            mentions,
            replied,
        },
    )
    .await;
    // `assert_send_chain` 返回时 `run_helix_send` 已停止 engine/WS 并删除 SQLite；远端频道关闭必须
    // 严格位于这个本地资源屏障之后，避免迟到帧在 cleanup 阶段触发二次 snapshot 错误。
    let cleanup = close_channel(&context, &channel_id).await;
    let evidence = match (result, cleanup) {
        (Ok(evidence), Ok(())) => evidence,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(format!("channel cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => {
            return Err(format!("{error}; channel cleanup failed: {cleanup_error}"));
        }
    };
    if message_type == "TASK_CARD" {
        let post_props = evidence
            .post_frame
            .pointer("/data/props")
            .ok_or_else(|| "TASK_CARD Go WS post has no props".to_string())?;
        assert_props_subset(post_props, &expected_props, "TASK_CARD Go WS post")?;
        let received_json: Value = serde_json::from_slice(evidence.received.0.as_ref())
            .map_err(|error| format!("TASK_CARD received event JSON failed: {error}"))?;
        let received_props = received_json
            .pointer("/data/props")
            .ok_or_else(|| "TASK_CARD received event has no props".to_string())?;
        assert_props_subset(received_props, &expected_props, "TASK_CARD received event")?;
        let stored_props = row_text(&evidence.row, "props")
            .ok_or_else(|| "TASK_CARD SQLite row has no props".to_string())?;
        let stored_props: Value = serde_json::from_str(&stored_props)
            .map_err(|error| format!("TASK_CARD SQLite props JSON failed: {error}"))?;
        assert_props_subset(&stored_props, &expected_props, "TASK_CARD SQLite row")?;
    }
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": "im_send_message",
            "effect": ["Persist", "NativeHttp", "NativeTransport"],
            "persist_before": "send_status=sending",
            "http_business": "observer received authoritative post after Go admission",
            "ws_action": "post",
            "projection": "im:post:received",
            "persist_after": "send_status=sent",
            "event_sink": "NativeEventSink:im:post:received",
            "sqlite_readback": {
                "table": "message",
                "temporary_id": evidence.temporary_id,
                "id": row_text(&evidence.row, "id"),
                "channel_id": row_text(&evidence.row, "channel_id"),
                "user_id": row_text(&evidence.row, "user_id"),
                "type": row_text(&evidence.row, "type"),
                "message": row_text(&evidence.row, "message"),
                "send_status": row_text(&evidence.row, "send_status"),
            },
            "projection_props": (message_type == "TASK_CARD")
                .then(|| evidence.post_frame.pointer("/data/props").cloned())
                .flatten(),
            "cleanup": {
                "status": "PASS",
                "remote_channel": channel_id,
                "engine_ws_sqlite": "closed by run_helix_send",
            },
            "corr_key": evidence.temporary_id,
        }),
    )?;
    Ok(evidence)
}

/// 通过 Helix public outbound command 触发 Java engagement 创建，并只接受 Go/WS→Helix
/// 完整消息投影作为成功证据；Java HTTP 受理本身不能替代真实 Post 回声。
pub(crate) async fn assert_java_engagement_create_chain(
    phase: &str,
    domain: &str,
    gate: &str,
    command: &'static str,
    endpoint: &str,
    payload: Value,
    expected_type: &str,
    expected_message: &str,
    expected_props_key: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "run-owned Go channel fixture could not be created",
                    "first_failing_layer": "Go channel/create",
                    "command": command,
                    "go_http": endpoint,
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };

    let mut observer =
        match start_real_helix_session_as(&context, &context.observer, &[channel.as_str()]).await {
            Ok(session) => session,
            Err(error) => {
                let cleanup = close_channel(&context, &channel).await;
                if let Err(cleanup_error) = cleanup {
                    return Err(format!("{error}; channel cleanup failed: {cleanup_error}"));
                }
                write_gate_evidence(
                    phase,
                    domain,
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "observer Helix session could not be started",
                        "first_failing_layer": "Helix native transport/auth",
                        "command": command,
                        "go_http": endpoint,
                        "error": error,
                    }),
                )?;
                return Ok(());
            }
        };
    let actor =
        match start_real_helix_session_as(&context, &context.actor, &[channel.as_str()]).await {
            Ok(session) => session,
            Err(error) => {
                let observer_cleanup = observer.close().await;
                let channel_cleanup = close_channel(&context, &channel).await;
                if let Err(cleanup_error) = observer_cleanup {
                    return Err(format!("{error}; observer cleanup failed: {cleanup_error}"));
                }
                if let Err(cleanup_error) = channel_cleanup {
                    return Err(format!("{error}; channel cleanup failed: {cleanup_error}"));
                }
                write_gate_evidence(
                    phase,
                    domain,
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "actor Helix session could not be started",
                        "first_failing_layer": "Helix native transport/auth",
                        "command": command,
                        "go_http": endpoint,
                        "error": error,
                    }),
                )?;
                return Ok(());
            }
        };

    let chain_result = async {
        let mut command_payload = payload.clone();
        if let Some(object) = command_payload.as_object_mut() {
            if command == "im_vote_create" {
                object
                    .entry("from_user_id".to_string())
                    .or_insert_with(|| json!(context.actor.user_id.clone()));
                object
                    .entry("from_user_name".to_string())
                    .or_insert_with(|| json!(context.actor.user_name.clone()));
                object.entry("source".to_string()).or_insert_with(|| {
                    json!({"id": channel, "type": "im"})
                });
                if let Some(source) = object.get_mut("source").and_then(Value::as_object_mut) {
                    source.insert("id".to_string(), json!(channel));
                    source.entry("type".to_string()).or_insert_with(|| json!("im"));
                }
                object.entry("org_ids".to_string()).or_insert_with(|| {
                    json!([
                        {"id": context.actor.user_id, "name": context.actor.user_name, "is_user": true, "org_id": "", "org_type": "USER", "org_name": ""},
                        {"id": context.observer.user_id, "name": context.observer.user_name, "is_user": true, "org_id": "", "org_type": "USER", "org_name": ""}
                    ])
                });
            } else if command == "im_average_publish" {
                object.insert("source".to_string(), json!({"id": channel, "type": "im"}));
                object.insert(
                    "members".to_string(),
                    json!([
                        {"user_id": context.actor.user_id, "user_name": context.actor.user_name},
                        {"user_id": context.observer.user_id, "user_name": context.observer.user_name}
                    ]),
                );
            }
        }
        let corr_key = command_payload
            .get("req_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| run_token("engagement-create"));
        if command_payload.get("req_id").is_none() {
            command_payload["req_id"] = Value::String(corr_key.clone());
        }
        actor.command(command, command_payload).await?;

        let post_frame = observer
            .wait_ws_action(
                "post",
                |frame| {
                    frame.pointer("/data/channelId").and_then(Value::as_str) == Some(channel.as_str())
                        && frame.pointer("/data/type").and_then(Value::as_str) == Some(expected_type)
                        && frame.pointer("/data/message").and_then(Value::as_str) == Some(expected_message)
                },
                Duration::from_secs(35),
            )
            .await?;
        let post_data = post_frame
            .get("data")
            .ok_or_else(|| format!("{gate} Java-created post frame has no data: {post_frame}"))?;
        let post_id = string_field(post_data, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} Java-created post frame has no server id: {post_frame}"))?;
        let props = post_data
            .get("props")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{gate} Java-created post frame has no props object: {post_frame}"))?;
        if !props.contains_key(expected_props_key) {
            return Err(format!(
                "{gate} Java-created post props omitted {expected_props_key}: {post_frame}"
            ));
        }
        let received = observer
            .wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["id", "postId", "post_id"]).as_deref()
                        == Some(post_id.as_str())
                        && data_string(envelope, &["type"]).as_deref() == Some(expected_type)
                },
                Duration::from_secs(35),
            )
            .await?;
        let received_json: Value = serde_json::from_slice(received.0.as_ref())
            .map_err(|error| format!("{gate} Java-created received event JSON failed: {error}"))?;
        let received_props = received_json
            .pointer("/data/props")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("{gate} Java-created received event has no props: {received_json}"))?;
        if !received_props.contains_key(expected_props_key) {
            return Err(format!(
                "{gate} Helix received projection omitted {expected_props_key}: {received_json}"
            ));
        }
        let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
        if row_text(&row, "type").as_deref() != Some(expected_type)
            || row_text(&row, "channel_id").as_deref() != Some(channel.as_str())
        {
            return Err(format!("{gate} SQLite Java-created message mismatch: {row:?}"));
        }
        let row_props = row_text(&row, "props")
            .ok_or_else(|| format!("{gate} SQLite Java-created message has no props"))?;
        let row_props: Value = serde_json::from_str(&row_props)
            .map_err(|error| format!("{gate} SQLite Java-created props JSON failed: {error}"))?;
        if row_props.get(expected_props_key).is_none() {
            return Err(format!(
                "{gate} SQLite Java-created message omitted {expected_props_key}: {row_props}"
            ));
        }
        Ok::<Value, String>(json!({
            "corr_key": corr_key,
            "channel_id": channel,
            "post_id": post_id,
            "post_frame": post_frame,
            "received": received_json,
            "sqlite": {
                "table": "message",
                "id": row_text(&row, "id"),
                "type": row_text(&row, "type"),
                "channel_id": row_text(&row, "channel_id"),
                "send_status": row_text(&row, "send_status"),
                "props_key": expected_props_key,
            },
        }))
    }
    .await;
    let actor_cleanup = actor.close().await;
    let observer_cleanup = observer.close().await;
    let channel_cleanup = close_channel(&context, &channel).await;
    let result = match (
        chain_result,
        actor_cleanup,
        observer_cleanup,
        channel_cleanup,
    ) {
        (Ok(details), Ok(()), Ok(()), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "PASS",
                json!({
                    "command": command,
                    "go_http": endpoint,
                    "http_business": "Java engagement command produced an authoritative Go post",
                    "ws_action": "post",
                    "projection": "im:post:received",
                    "persist_after": "observer SQLite message projection",
                    "event_sink": "NativeEventSink:im:post:received",
                    "event_sink_required": true,
                    "sqlite_required": true,
                    "sqlite_readback": details.get("sqlite").cloned().unwrap_or(Value::Null),
                    "cleanup": {"status": "PASS", "remote_channel": details["channel_id"]},
                    "corr_key": details["corr_key"],
                    "details": details,
                }),
            )?;
            Ok(())
        }
        (chain, actor, observer, channel) => {
            let error = match chain {
                Err(error) => error,
                Ok(_) => "Java engagement chain did not produce PASS evidence".to_string(),
            };
            let error = match actor {
                Ok(()) => error,
                Err(cleanup_error) => format!("{error}; actor cleanup failed: {cleanup_error}"),
            };
            let error = match observer {
                Ok(()) => error,
                Err(cleanup_error) => format!("{error}; observer cleanup failed: {cleanup_error}"),
            };
            let error = match channel {
                Ok(()) => error,
                Err(cleanup_error) => format!("{error}; channel cleanup failed: {cleanup_error}"),
            };
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "Java engagement command did not produce the complete Go/WS/Helix projection",
                    "first_failing_layer": "Java HTTP → Java IMService → Go posts/create → WS/Helix projection",
                    "command": command,
                    "go_http": endpoint,
                    "expected_type": expected_type,
                    "expected_props_key": expected_props_key,
                    "error": error,
                }),
            )?;
            Ok(())
        }
    };
    result
}

/// 用 native HTTP 的一次性故障开关制造真实 failed row，再走 `im_retry_send` 同一
/// temporaryId；该 seam 只存在测试适配器，生产调用不会被注入故障。
pub(crate) async fn assert_retry_send_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "run-owned retry channel fixture could not be created",
                    "first_failing_layer": "Go channel/create",
                    "command": "im_send_message → im_retry_send",
                    "go_http": "/api/cses/posts/create",
                    "error": error,
                }),
            )?;
            return Err(error);
        }
    };
    let mut session =
        match start_real_helix_session_as(&context, &context.actor, &[channel.as_str()]).await {
            Ok(session) => session,
            Err(error) => {
                let cleanup = close_channel(&context, &channel).await;
                if let Err(cleanup_error) = cleanup {
                    return Err(format!("{error}; channel cleanup failed: {cleanup_error}"));
                }
                write_gate_evidence(
                    phase,
                    domain,
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "real Helix retry session could not be started",
                        "first_failing_layer": "Helix native transport/auth",
                        "command": "im_send_message → im_retry_send",
                        "go_http": "/api/cses/posts/create",
                        "error": error,
                    }),
                )?;
                return Err(error);
            }
        };

    let marker = resource_name("retry");
    let first_req_id = run_token("retry-first");
    let retry_req_id = run_token("retry-second");
    let mut attempted_temporary_id = None;
    let mut stage = "initial_send";
    let mut retry_ws_metadata = Value::Null;
    let first_result = async {
        helix_driver_native::configure_wdio_fail_next_post_create(true);
        session
            .command(
                "im_send_message",
                json!({
                    "channel_id": channel,
                    "type": "TEXT",
                    "text": marker,
                    "props": {},
                    "viewers": ["all"],
                    "mentions": [],
                    "req_id": first_req_id,
                }),
            )
            .await?;
        let sending = session
            .wait_event(
                "im:post:sending",
                |_| true,
                Duration::from_secs(20),
            )
            .await?;
        let sending_json: Value = serde_json::from_slice(sending.0.as_ref())
            .map_err(|error| format!("{gate} retry sending event JSON failed: {error}"))?;
        let temporary_id = data_string(&sending_json, &["temporaryId", "temporary_id"])
            .ok_or_else(|| format!("{gate} retry sending event has no temporaryId"))?;
        attempted_temporary_id = Some(temporary_id.clone());
        stage = "failed_row";
        let failed = session
            .wait_event(
                "im:post:send-failed",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(20),
            )
            .await?;
        let failed_json: Value = serde_json::from_slice(failed.0.as_ref())
            .map_err(|error| format!("{gate} retry failed event JSON failed: {error}"))?;
        let failed_row = wait_message_status(
            &session.storage,
            &temporary_id,
            &["unsend", "failed"],
            Duration::from_secs(8),
        )
        .await?;
        helix_driver_native::configure_wdio_fail_next_post_create(false);
        session
            .command(
                "im_retry_send",
                json!({"message_id": temporary_id, "req_id": retry_req_id}),
            )
            .await?;
        let retry_sending = session
            .wait_event(
                "im:post:sending",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(20),
            )
            .await?;
        stage = "retry_ws_post";
        let retry_frame = session
            .wait_ws_action(
                "post",
                |frame| {
                    frame.pointer("/data/temporaryId").and_then(Value::as_str)
                        == Some(temporary_id.as_str())
                        && frame.pointer("/data/channelId").and_then(Value::as_str)
                            == Some(channel.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        retry_ws_metadata = json!({
            "id": retry_frame.pointer("/data/id"),
            "temporaryId": retry_frame.pointer("/data/temporaryId"),
            "channelId": retry_frame.pointer("/data/channelId"),
            "event_seq": retry_frame.get("event_seq"),
            "data_event_seq": retry_frame.pointer("/data/event_seq"),
            "data_seq": retry_frame.pointer("/data/seq"),
        });
        stage = "retry_received";
        let received = session.wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let received_json: Value = serde_json::from_slice(received.0.as_ref())
            .map_err(|error| format!("{gate} retry received event JSON failed: {error}"))?;
        let sent_row = wait_message_status(
            &session.storage,
            &temporary_id,
            &["sent"],
            Duration::from_secs(8),
        )
        .await?;
        let duplicate = session
            .wait_ws_action(
                "post",
                |frame| {
                    frame.pointer("/data/temporaryId").and_then(Value::as_str)
                        == Some(temporary_id.as_str())
                },
                Duration::from_millis(1_200),
            )
            .await;
        if duplicate.is_ok() {
            return Err(format!("{gate} retry emitted duplicate post frame for {temporary_id}"));
        }
        Ok::<Value, String>(json!({
            "first_request": first_req_id,
            "retry_request": retry_req_id,
            "temporary_id": temporary_id,
            "failed_event": failed_json,
            "failed_row": {
                "send_status": row_text(&failed_row, "send_status"),
                "channel_id": row_text(&failed_row, "channel_id"),
                "message": row_text(&failed_row, "message"),
            },
            "retry_sending": serde_json::from_slice::<Value>(retry_sending.0.as_ref()).unwrap_or(Value::Null),
            "retry_frame": retry_frame,
            "received": received_json,
            "sent_row": {
                "send_status": row_text(&sent_row, "send_status"),
                "id": row_text(&sent_row, "id"),
                "channel_id": row_text(&sent_row, "channel_id"),
                "message": row_text(&sent_row, "message"),
            },
            "duplicate_post": false,
        }))
    }
    .await;
    helix_driver_native::configure_wdio_fail_next_post_create(false);
    // 失败快照必须在关闭/删除SQLite之前采集，仅记录状态与关联身份。
    let failure_storage = if first_result.is_err() {
        if let Some(temporary_id) = &attempted_temporary_id {
            match scan_message_row(&session.storage, temporary_id).await {
                Ok(row) => {
                    json!({"id": row_text(&row, "id"), "send_status": row_text(&row, "send_status"),
                    "channel_id": row_text(&row, "channel_id"), "temporary_id": temporary_id})
                }
                Err(error) => json!({"error": error}),
            }
        } else {
            Value::Null
        }
    } else {
        Value::Null
    };
    let session_cleanup = session.close().await;
    let channel_cleanup = close_channel(&context, &channel).await;
    match (first_result, session_cleanup, channel_cleanup) {
        (Ok(details), Ok(()), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "PASS",
                json!({
                    "command": "im_send_message → im_retry_send",
                    "go_http": "/api/cses/posts/create",
                    "http_business": "first post/create was intentionally failed, retry reached Go authoritative post",
                    "projection": "im:post:send-failed → im:post:received",
                    "persist_after": "unsend/failed → sent",
                    "event_sink": "NativeEventSink terminal retry events",
                    "event_sink_required": true,
                    "sqlite_required": true,
                    "sqlite_readback": {
                        "table": "message",
                        "temporary_id": details["temporary_id"],
                        "failed": details["failed_row"],
                        "sent": details["sent_row"]
                    },
                    "cleanup": {"status": "PASS", "remote_channel": channel},
                    "corr_key": details["temporary_id"],
                    "details": details,
                }),
            )?;
            Ok(())
        }
        (result, session, channel) => {
            let error = match result {
                Err(error) => error,
                Ok(_) => "retry chain did not produce PASS evidence".to_string(),
            };
            let error = match session {
                Ok(()) => error,
                Err(cleanup_error) => format!("{error}; session cleanup failed: {cleanup_error}"),
            };
            let error = match channel {
                Ok(()) => error,
                Err(cleanup_error) => format!("{error}; channel cleanup failed: {cleanup_error}"),
            };
            write_gate_evidence(
                phase,
                domain,
                gate,
                "NOT_VERIFIED",
                json!({
                    "reason": "real failed-row/retry attempt did not complete all terminal assertions",
                    "stage": stage,
                    "retry_ws_metadata": retry_ws_metadata,
                    "failure_storage": failure_storage,
                    "corr_key": attempted_temporary_id,
                    "cleanup": {"session": "attempted", "channel": "attempted"},
                    "first_failing_layer": "Helix retry state or Go posts/create/WS",
                    "command": "im_send_message → im_retry_send",
                    "go_http": "/api/cses/posts/create",
                    "error": error,
                }),
            )?;
            Err(error)
        }
    }
}

/// 用真实媒体字节跑 Helix 的 Java 票据→OSS PUT→Local Complete→Go `posts/create` 链。
///
/// `run_helix_send` 会从 `mediaInput` 触发 Helix 自己的 Prepare/UploadFile 状态机；这里
/// 只负责生成受控临时文件、校验最终稳定引用，并在证据写完后删除本轮文件。
pub(crate) async fn assert_media_reference_send(
    phase: &str,
    domain: &str,
    gate: &str,
    message_type: &'static str,
    text: String,
    props: Value,
    extra: Value,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let (staged_props, staged) = match stage_media_inputs(props, gate).await {
        Ok(value) => value,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "Helix mediaInput staging could not create a valid local byte source",
                    "first_failing_layer": "Native Host media staging",
                    "media_plane": {"status": "BLOCKED", "contract": "mediaInput -> Java /oss/getPresignedUrl -> OSS PUT"},
                    "error": error,
                    "extra": extra,
                }),
            )?;
            return Ok(());
        }
    };
    let staged_evidence = staged_media_evidence(&staged);
    let staged_paths = staged
        .iter()
        .map(|fixture| fixture.local_path.clone())
        .collect::<Vec<_>>();
    let channel = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            remove_staged_media_files(&staged_paths).await;
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Go channel could not be created for media reference send",
                    "first_failing_layer": "Go channel/create",
                    "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
                    "go_role": "posts/create after Local Complete",
                    "media_plane": {"status": "NOT_RUN", "fixtures": staged_evidence},
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let result = run_helix_send(
        &context,
        &channel,
        SendIntent {
            message_type,
            text,
            props: staged_props,
            viewers: vec!["all".to_string()],
            mentions: Vec::new(),
            replied: None,
        },
        15_000,
    )
    .await;
    let cleanup = close_channel(&context, &channel).await;
    if let Err(error) = cleanup {
        remove_staged_media_files(&staged_paths).await;
        return Err(format!("media reference channel cleanup failed: {error}"));
    }
    remove_staged_media_files(&staged_paths).await;

    let evidence = match result {
        Ok(evidence) => evidence,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "stable media reference did not complete the real Go post chain",
                    "first_failing_layer": "Helix media Prepare/UploadFile or Go posts/create/WS/projection",
                    "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
                    "go_role": "posts/create after Local Complete",
                    "media_plane": {"status": "BLOCKED", "fixtures": staged_evidence},
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let received: Value = serde_json::from_slice(evidence.received.0.as_ref())
        .map_err(|error| format!("{gate} decode received media event failed: {error}"))?;
    let actual_props = received
        .pointer("/data/props")
        .ok_or_else(|| format!("{gate} received event has no data.props"))?;
    if let Err(error) = assert_staged_media_projection(actual_props, &staged, gate) {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "Go/WS/Helix chain completed but media reference metadata did not round-trip",
                "first_failing_layer": "Go post props or Helix received projection",
                "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
                "go_role": "posts/create after Local Complete",
                "media_plane": {"status": "PASS", "fixtures": staged_evidence},
                "verified_subchain": [
                    "Effect::Http POST Java /oss/getPresignedUrl",
                    "Effect::UploadFile PUT signed OSS URL",
                    "PersistAtomic Local Complete + pending_media delete",
                    "Tick::Command(im_send_message)",
                    "im:post:sending",
                    "SQLite send_status=sending",
                    "Go posts/create SUCCESS",
                    "Go WS action=post",
                    "im:post:received",
                    "SQLite send_status=sent",
                    "NativeEventSink/im:__bus__"
                ],
                "error": error,
                "temporary_id": evidence.temporary_id,
                "extra": extra,
            }),
        )?;
        return Ok(());
    }
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "reason": "real Helix media state machine completed Java ticket, OSS PUT, Local Complete and Go post consumption",
            "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
            "go_role": "posts/create after Local Complete",
            "http_business": "Java /oss/getPresignedUrl and signed OSS PUT completed before Go posts/create SUCCESS",
            "projection": "im:post:received",
            "persist_after": "SQLite send_status=sent and pending_media journal removed",
            "event_sink": "NativeEventSink:im:post:received",
            "event_sink_required": true,
            "sqlite_required": true,
            "sqlite_readback": {
                "table": "message",
                "temporary_id": evidence.temporary_id,
                "id": row_text(&evidence.row, "id"),
                "channel_id": row_text(&evidence.row, "channel_id"),
                "user_id": row_text(&evidence.row, "user_id"),
                "type": row_text(&evidence.row, "type"),
                "message": row_text(&evidence.row, "message"),
                "send_status": row_text(&evidence.row, "send_status"),
            },
            "media_plane": {"status": "PASS", "fixtures": staged_evidence},
            "verified_subchain": [
                "Effect::Http POST Java /oss/getPresignedUrl",
                "Effect::UploadFile PUT signed OSS URL",
                "PersistAtomic Local Complete + pending_media delete",
                "Tick::Command(im_send_message)",
                "im:post:sending",
                "SQLite send_status=sending",
                "Go posts/create SUCCESS",
                "Go WS action=post",
                "im:post:received",
                "SQLite send_status=sent",
                "NativeEventSink/im:__bus__"
            ],
            "stable_reference_round_trip": true,
            "temporary_id": evidence.temporary_id,
            "cleanup": {"status": "PASS", "remote_channel": channel, "local_fixtures": "removed"},
            "extra": extra,
        }),
    )?;
    Ok(())
}

/// 以 Go 已存在而本地为空的消息验证 latest Timeline 的 HTTP→cache Persist→SQLite read-back 链。
pub(crate) async fn assert_latest_timeline_gate(gate: &str) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let marker = resource_name("timeline-latest");
        let (_response, post_frame) = direct_post_as(
            &context.actor,
            &context.observer,
            &channel,
            "TEXT",
            &marker,
            json!({}),
            json!(["all"]),
            json!([]),
            71,
        )
        .await?;
        let temporary_id = extract_id(&post_frame, &["temporaryId", "temporary_id"])
            .ok_or_else(|| format!("{gate} seed post has no temporaryId: {post_frame}"))?;
        let post_id = extract_id(&post_frame, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} seed post has no server id: {post_frame}"))?;
        let visibility_http = go_http(&context.actor).await?;
        let visibility_deadline = Instant::now() + Duration::from_secs(20);
        let mut visible = false;
        while Instant::now() < visibility_deadline {
            if let Ok(observation) = request_json(
                &visibility_http,
                "POST",
                &format!("{}/posts/get", go_http_base()),
                Some(json!({"postIds": [post_id.clone()]})),
                "timeline-seed-readback",
            )
            .await
            {
                if let Ok(body) = require_success(observation, "posts/get") {
                    let encoded = serde_json::to_string(&body).unwrap_or_default();
                    if encoded.contains(marker.as_str()) {
                        visible = true;
                        break;
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        if !visible {
            write_gate_evidence(
                "Phase2",
                "Timeline",
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "Go posts/create returned an outbound post frame, but the same post was not readable from real Go posts/get within 20s",
                    "first_failing_layer": "Go producer/consumer or remote post persistence",
                    "post_id": post_id,
                    "temporary_id": temporary_id,
                    "runtime": go_http_base(),
                }),
            )?;
            return Err(format!(
                "{gate} seed post is not visible in Go posts/get after asynchronous create; post_id={post_id}"
            ));
        }
        let mut session = start_real_helix_session(&context, &[channel.as_str()]).await?;
        let window_token = resource_name("timeline-window");
        session
            .command(
                "im_query_messages_by_channel",
                json!({
                    "channel_id": channel,
                    "pageSize": 20,
                    "windowToken": window_token,
                }),
            )
            .await?;
        let event = session
            .wait_event(
                "im:timeline:ready",
                |envelope| {
                    envelope.pointer("/data/messages").is_some()
                        && data_string(envelope, &["channelId", "channel_id"]).is_some()
                },
                Duration::from_secs(25),
            )
            .await?;
        let event_json: Value = serde_json::from_slice(event.0.as_ref())
            .map_err(|error| format!("{gate} timeline event JSON failed: {error}"))?;
        let messages = event_json
            .pointer("/data/messages")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{gate} timeline event has no messages: {event_json}"))?;
        if !messages.iter().any(|message| {
            string_field(message, &["text", "message"]).as_deref() == Some(marker.as_str())
        }) {
            return Err(format!(
                "{gate} timeline event omitted seeded marker: {event_json}"
            ));
        }
        let row = scan_message_row(&session.storage, &temporary_id).await?;
        if row_text(&row, "send_status").as_deref() != Some("sent") {
            return Err(format!("{gate} cache row is not sent: {row:?}"));
        }
        write_gate_evidence(
            "Phase2",
            "Timeline",
            gate,
            "PASS",
            json!({
                "command": "im_query_messages_by_channel",
                "go_http": "/api/cses/posts/getLatestPost",
                "http_business": "Go posts/get status=SUCCESS and marker read-back",
                "ws_action": "post seed frame",
                "pipeline": ["Tick::Command", "Helix local Scan miss", "Go HTTP", "Persist cache", "SQLite read-back", "im:timeline:ready"],
                "channel_id": channel,
                "post_id": post_id,
                "temporary_id": temporary_id,
                "window_token": window_token,
                "projection": "im:timeline:ready",
                "persist_after": "message.send_status=sent",
                "event_sink": "NativeEventSink:im:timeline:ready",
                "event_sink_required": true,
                "sqlite_required": true,
                "sqlite_readback": {"table": "message", "temporary_id": temporary_id, "send_status": row_text(&row, "send_status")},
                "cleanup": {"status": "PASS", "remote_channel": channel},
                "corr_key": temporary_id,
                "event": event_json,
            }),
        )?;
        session.close().await
    }
    .await;
    finish_run_owned_channel(&context, &channel, result).await
}

/// 执行 G11g exact-by-id 读链，并保证 read session 与 run-owned 频道按顺序收尾。
pub(crate) async fn assert_exact_timeline_gate(gate: &str) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let (_response, frame) = direct_post_as(
            &context.actor,
            &context.observer,
            &channel,
            "TEXT",
            &resource_name("exact-post"),
            json!({}),
            json!(["all"]),
            json!([]),
            72,
        )
        .await?;
        let post_id = extract_id(&frame, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} seed post has no id: {frame}"))?;
        let temporary_id = extract_id(&frame, &["temporaryId", "temporary_id"])
            .ok_or_else(|| format!("{gate} seed post has no temporary id: {frame}"))?;
        let mut session = start_real_helix_session(&context, &[channel.as_str()]).await?;
        let session_result = async {
            let req_id = run_token("g11g-exact");
            session
                .command(
                    "im_get_posts",
                    json!({"post_ids": [post_id.clone()], "req_id": req_id}),
                )
                .await?;
            let event = session
                .wait_event(
                    "im:read:result",
                    |envelope| {
                        data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                    },
                    Duration::from_secs(25),
                )
                .await?;
            let event_json: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("{gate} event JSON failed: {error}"))?;
            let body = event_json
                .pointer("/data/body")
                .ok_or_else(|| format!("{gate} result has no body: {event_json}"))?;
            let posts = body
                .get("posts")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{gate} exact result has no posts array: {body}"))?;
            if posts.is_empty() {
                write_gate_evidence(
                    "Phase2",
                    "Timeline",
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "Go posts/get returned SUCCESS-shaped exact result but no seeded post",
                        "first_failing_layer": "Go producer/consumer or remote post persistence",
                        "post_id": post_id,
                        "event": event_json,
                    }),
                )?;
                return Ok::<(), String>(());
            }
            if !posts.iter().any(|post| {
                extract_id(post, &["id", "postId", "serverId"]).as_deref()
                    == Some(post_id.as_str())
            }) {
                return Err(format!(
                    "{gate} exact post result omitted requested server id: {body}"
                ));
            }
            let row = scan_message_row(&session.storage, &temporary_id).await?;
            if row_text(&row, "send_status").as_deref() != Some("sent") {
                return Err(format!("{gate} durable row is not sent: {row:?}"));
            }
            write_gate_evidence(
                "Phase2",
                "Timeline",
                gate,
                "PASS",
                json!({
                    "command": "im_get_posts",
                    "go_http": "/api/cses/posts/get",
                    "http_business": "Go posts/get status=SUCCESS",
                    "projection": "im:read:result",
                    "persist_after": "message exact-post row read-back",
                    "sqlite_required": true,
                    "sqlite_readback": {"table": "message", "temporary_id": temporary_id, "send_status": row_text(&row, "send_status")},
                    "cleanup": {"status": "PASS", "remote_channel": channel},
                    "corr_key": temporary_id,
                    "terminal_event": "im:read:result",
                    "post_id": post_id,
                    "temporary_id": temporary_id,
                    "event": event_json,
                }),
            )?;
            Ok::<(), String>(())
        }
        .await;
        let session_cleanup = session.close().await;
        match (session_result, session_cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(format!("Helix session cleanup failed: {error}")),
            (Err(error), Err(cleanup_error)) => Err(format!(
                "{error}; Helix session cleanup failed: {cleanup_error}"
            )),
        }
    }
    .await;
    finish_run_owned_channel(&context, &channel, result).await
}

/// 在既有 run-owned channel 上执行一条真实 command，并等待唯一 terminal DomainEvent。
///
/// 该 helper 只接受真实 native engine 结果；对 `im:read:result` 还会检查 Go body 的
/// `status=SUCCESS`，避免把 HTTP 200、空事件或本地 admission 当作业务完成。
pub(crate) async fn assert_real_command_event<F>(
    context: &AuthContext,
    phase: &str,
    domain: &str,
    gate: &str,
    channel_id: &str,
    command: &'static str,
    endpoint: &str,
    payload: Value,
    event_name: &str,
    predicate: F,
) -> Result<Value, String>
where
    F: Fn(&Value) -> bool,
{
    let mut session = start_real_helix_session(context, &[channel_id]).await?;
    session.command(command, payload.clone()).await?;
    let event = session
        .wait_event(event_name, predicate, Duration::from_secs(25))
        .await?;
    let event_json: Value = serde_json::from_slice(event.0.as_ref())
        .map_err(|error| format!("{gate} event JSON failed: {error}"))?;
    if event_name == "im:read:result" {
        let body = event_json
            .pointer("/data/body")
            .ok_or_else(|| format!("{gate} read result has no Go body: {event_json}"))?;
        if body.get("status").and_then(Value::as_str) != Some("SUCCESS") {
            return Err(format!("{gate} Go business failure: {body}"));
        }
    }
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": command,
            "go_http": endpoint,
            "terminal_event": event_name,
            "payload_keys": payload.as_object().map(|object| object.keys().collect::<Vec<_>>()),
            "event": event_json,
        }),
    )?;
    session.close().await?;
    Ok(event_json)
}

/// 为远端读命令保留一次真实 Helix/Go 尝试；外部 producer、权限或远端数据缺失时写入可审计阻塞。
pub(crate) async fn assert_real_read_or_blocked<F>(
    context: &AuthContext,
    phase: &str,
    domain: &str,
    gate: &str,
    channel_id: &str,
    command: &'static str,
    endpoint: &str,
    payload: Value,
    predicate: F,
) -> Result<(), String>
where
    F: Fn(&Value) -> bool,
{
    assert_real_read_or_blocked_as(
        context,
        &context.actor,
        phase,
        domain,
        gate,
        channel_id,
        command,
        endpoint,
        payload,
        predicate,
    )
    .await
}

/// 以指定真实身份执行远端读；Search 等场景需要用 ES 已授权的会话而非默认 actor。
pub(crate) async fn assert_real_read_or_blocked_as<F>(
    context: &AuthContext,
    persona: &Persona,
    phase: &str,
    domain: &str,
    gate: &str,
    channel_id: &str,
    command: &'static str,
    endpoint: &str,
    payload: Value,
    predicate: F,
) -> Result<(), String>
where
    F: Fn(&Value) -> bool,
{
    let mut session = match start_real_helix_session_as(context, persona, &[channel_id]).await {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Helix session could not be started",
                    "first_failing_layer": "Helix native transport/auth precondition",
                    "command": command,
                    "go_http": endpoint,
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    if let Err(error) = session.command(command, payload.clone()).await {
        let _ = session.close().await;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "real Helix command was not admitted",
                "first_failing_layer": "Helix Tick ingress",
                "command": command,
                "go_http": endpoint,
                "error": error,
            }),
        )?;
        return Ok(());
    }
    let expected_req_id = payload
        .get("req_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let event = match session
        .wait_event(
            "im:read:result",
            |envelope| {
                expected_req_id
                    .as_deref()
                    .map(|req_id| data_string(envelope, &["req_id"]).as_deref() == Some(req_id))
                    .unwrap_or_else(|| data_string(envelope, &["req_id"]).is_some())
            },
            Duration::from_secs(25),
        )
        .await
    {
        Ok(event) => event,
        Err(error) => {
            let _ = session.close().await;
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Go read result did not arrive",
                    "first_failing_layer": "Go route/session/remote persistence or Helix read relay",
                    "command": command,
                    "go_http": endpoint,
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let event_json: Value = serde_json::from_slice(event.0.as_ref())
        .map_err(|error| format!("{gate} read event JSON failed: {error}"))?;
    let body = event_json.pointer("/data/body");
    let business_success = body
        .and_then(|body| body.get("status"))
        .and_then(Value::as_str)
        .map(|status| status == "SUCCESS")
        .unwrap_or(true);
    if event_json.pointer("/data/error").is_some() || !business_success {
        let _ = session.close().await;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "real Go read returned a business or transport error",
                "first_failing_layer": "Go route/auth/remote store",
                "command": command,
                "go_http": endpoint,
                "event": event_json,
            }),
        )?;
        return Ok(());
    }
    if !predicate(&event_json) {
        let _ = session.close().await;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "real Go read result arrived but did not satisfy the Gate response predicate",
                "first_failing_layer": "Go response schema or Helix read projection",
                "command": command,
                "go_http": endpoint,
                "event": event_json,
            }),
        )?;
        return Ok(());
    }
    session
        .close()
        .await
        .map_err(|error| format!("{gate} Helix session cleanup failed: {error}"))?;
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": command,
            "go_http": endpoint,
            "http_business": "Go read body status=SUCCESS",
            "projection": "im:read:result",
            "persist_after": "terminal read projection received with req_id",
            "cleanup": {"status": "PASS", "transport": "closed"},
            "corr_key": expected_req_id.clone().unwrap_or_else(|| gate.to_string()),
            "terminal_event": "im:read:result",
            "payload_keys": payload.as_object().map(|object| object.keys().collect::<Vec<_>>()),
            "event": event_json,
        }),
    )?;
    Ok(())
}

/// 执行一个真实 HTTP mutation，并把 Go WS/Helix terminal projection 作为唯一成功证据。
pub(crate) async fn assert_real_mutation_or_blocked<F>(
    context: &AuthContext,
    phase: &str,
    domain: &str,
    gate: &str,
    channel_id: &str,
    command: &'static str,
    endpoint: &str,
    payload: Value,
    event_name: &str,
    predicate: F,
) -> Result<(), String>
where
    F: Fn(&Value) -> bool,
{
    // leave 会移除调用者的成员行；让 observer 执行该动作，保留 creator 供最终清理。
    let mut session = match if command == "im_channel_leave" {
        start_real_helix_session_as(context, &context.observer, &[channel_id]).await
    } else {
        start_real_helix_session(context, &[channel_id]).await
    } {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({"reason": "real Helix session unavailable", "first_failing_layer": "Helix native transport/auth", "command": command, "go_http": endpoint, "error": error}),
            )?;
            return Ok(());
        }
    };
    insert_channel_fixture(
        &session.storage,
        channel_id,
        &context.actor.company_id,
        &context.actor.user_id,
        "P",
        "",
        &[
            context.actor.user_id.as_str(),
            context.observer.user_id.as_str(),
            context.third.user_id.as_str(),
        ],
    )
    .await
    .map_err(|error| format!("{gate} local channel fixture failed: {error}"))?;
    if let Err(error) = session.command(command, payload.clone()).await {
        let _ = session.close().await;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({"reason": "real mutation command was not admitted", "first_failing_layer": "Helix Tick ingress", "command": command, "go_http": endpoint, "error": error}),
        )?;
        return Ok(());
    }
    if matches!(
        command,
        "im_channel_close" | "im_channel_leave" | "im_channel_member_change"
    ) {
        retire_phase_pool_channel(channel_id).await;
    }
    let event = match session
        .wait_event(
            event_name,
            |envelope| predicate(envelope),
            Duration::from_secs(25),
        )
        .await
    {
        Ok(event) => event,
        Err(error) => {
            let _ = session.close().await;
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({"reason": "Go mutation produced no terminal WS/Helix projection", "first_failing_layer": "Go route/producer/WS or Helix PersistOk", "command": command, "go_http": endpoint, "event": event_name, "error": error}),
            )?;
            return Ok(());
        }
    };
    let event_json: Value = serde_json::from_slice(event.0.as_ref())
        .map_err(|error| format!("{gate} mutation event JSON failed: {error}"))?;
    // 关闭频道的 durable 语义是 tombstone/row removal；其 event data 仍是 SQLite 读回证据。
    let sqlite_readback = match scan_channel_projection(&session.storage, channel_id).await {
        Ok(row) => json!({
            "table": "channel",
            "id": row_text(&row, "id"),
            "type": row_text(&row, "type"),
            "close": row_text(&row, "close"),
            "display_name": row_text(&row, "display_name")
        }),
        Err(error) if event_name == "im:channel:closed" => json!({
            "table": "channel",
            "id": channel_id,
            "row_present": false,
            "tombstone": event_json.pointer("/data").cloned().unwrap_or(Value::Null),
            "projection_error": error
        }),
        Err(error) => {
            let _ = session.close().await;
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "terminal mutation event arrived without a local channel read-back",
                    "first_failing_layer": "Helix PersistOk or Native SQLite projection",
                    "command": command,
                    "go_http": endpoint,
                    "event": event_json,
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    session
        .close()
        .await
        .map_err(|error| format!("{gate} Helix session cleanup failed: {error}"))?;
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": command,
            "go_http": endpoint,
            "http_business": "Go mutation reached authoritative terminal projection",
            "projection": event_name,
            "persist_after": "terminal DomainEvent received",
            "event_sink": "NativeEventSink canonical DomainEvent",
            "event_sink_required": true,
            "cleanup": {"status": "PASS", "transport": "closed"},
            "corr_key": payload.get("req_id").and_then(Value::as_str).unwrap_or(channel_id),
            "terminal_event": event_name,
            "channel_id": channel_id,
            "sqlite_required": true,
            "sqlite_readback": sqlite_readback,
            "event": event_json,
        }),
    )?;
    Ok(())
}

/// 等待指定置顶状态的 canonical projection，并核对同一状态已落入 SQLite。
async fn wait_pinned_projection(
    session: &mut RealHelixSession,
    gate: &str,
    post_id: &str,
    expected_pinned: bool,
) -> Result<(Value, Value), String> {
    let updated = session
        .wait_event(
            "im:post:updated",
            |envelope| {
                data_string(envelope, &["id", "postId", "post_id"]).as_deref() == Some(post_id)
                    && envelope
                        .pointer("/data/props/pinned")
                        .and_then(Value::as_bool)
                        == Some(expected_pinned)
            },
            Duration::from_secs(25),
        )
        .await?;
    let updated_json: Value = serde_json::from_slice(updated.0.as_ref())
        .map_err(|error| format!("{gate} update event JSON failed: {error}"))?;
    // pin WS 帧只携带服务端 post id；发送落库初期则以 temporary_id 为主键，
    // 因此必须使用同一消息的 id/temporary_id 双键精确回读，且等待 PersistFire 收敛。
    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last_props = Value::Null;
    let props_json = loop {
        if let Ok(row) = scan_message_row_by_id_or_temporary(&session.storage, post_id).await {
            if let Some(props) = row_text(&row, "props") {
                let parsed: Value = serde_json::from_str(&props)
                    .map_err(|error| format!("{gate} pin props JSON failed: {error}"))?;
                last_props = parsed.clone();
                if parsed.get("pinned").and_then(Value::as_bool) == Some(expected_pinned) {
                    break parsed;
                }
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "{gate} SQLite pinned patch mismatch: expected={expected_pinned}, last={last_props}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    Ok((updated_json, props_json))
}

/// 在同一真实 native 会话中先发送消息再执行 post mutation，核对 WS projection 与 SQLite patch。
pub(crate) async fn assert_post_pin_chain(
    phase: &str,
    domain: &str,
    gate: &str,
    command: &'static str,
    endpoint: &str,
    expected_pinned: bool,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let mut attempted_post_id = None::<String>;
    let mut stage = "session_setup";
    let mut attempted_temporary_id = None;
    let mut session = match start_real_helix_session(&context, &[channel.as_str()]).await {
        Ok(session) => session,
        Err(error) => {
            let cleanup = close_channel(&context, &channel).await;
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Helix session unavailable",
                    "first_failing_layer": "Helix native transport/auth",
                    "command": command,
                    "go_http": endpoint,
                    "error": error,
                }),
            )?;
            return Err(format!("{error}; channel_cleanup={cleanup:?}"));
        }
    };
    let result = async {
        stage = "seed_sending";
        session
            .command(
                "im_send_message",
                json!({
                    "channel_id": channel,
                    "type": "TEXT",
                    "text": resource_name("pin-seed"),
                    "props": {},
                    "viewers": ["all"],
                    "mentions": [],
                }),
            )
            .await?;
        let sending = session
            .wait_event(
                "im:post:sending",
                |envelope| data_string(envelope, &["temporaryId", "temporary_id"]).is_some(),
                Duration::from_secs(20),
            )
            .await?;
        let sending_json: Value = serde_json::from_slice(sending.0.as_ref())
            .map_err(|error| format!("{gate} sending event JSON failed: {error}"))?;
        let actual_temporary_id = data_string(&sending_json, &["temporaryId", "temporary_id"])
            .ok_or_else(|| format!("{gate} sending event has no temporaryId"))?;
        attempted_temporary_id = Some(actual_temporary_id.clone());
        stage = "seed_received";
        let _sending_row = scan_message_row(&session.storage, &actual_temporary_id).await?;
        let received = session
            .wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(actual_temporary_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let received_json: Value = serde_json::from_slice(received.0.as_ref())
            .map_err(|error| format!("{gate} received event JSON failed: {error}"))?;
        let post_id = data_string(&received_json, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} received event has no server post id"))?;
        attempted_post_id = Some(post_id.clone());
        let _sent_row = wait_message_status(
            &session.storage,
            &actual_temporary_id,
            &["sent"],
            Duration::from_secs(8),
        )
        .await?;
        stage = "seed_remote_readback";
        // Go 的 post WS 回声先于异步 producer durable commit；pin mutation 必须等远端可读。
        wait_remote_post_visible(&context, &post_id, None, gate).await?;
        // 取消置顶必须先建立同一 post 的置顶状态；Go 对不存在的 pin 是幂等 no-op，不会发 WS。
        stage = "pin_mutation";
        let (updated_json, props_json, setup_projection) = if expected_pinned {
            session
                .command(
                    command,
                    json!({
                        "channel_id": channel,
                        "post_id": post_id,
                    }),
                )
                .await?;
            let (updated_json, props_json) =
                wait_pinned_projection(&mut session, gate, &post_id, expected_pinned).await?;
            (updated_json, props_json, None)
        } else {
            session
                .command(
                    "im_set_message_top",
                    json!({
                        "channel_id": channel,
                        "post_id": post_id,
                    }),
                )
                .await?;
            let (pin_event, pin_props) =
                wait_pinned_projection(&mut session, gate, &post_id, true).await?;
            session
                .command(
                    command,
                    json!({
                        "channel_id": channel,
                        "post_id": post_id,
                    }),
                )
                .await?;
            let (updated_json, props_json) =
                wait_pinned_projection(&mut session, gate, &post_id, false).await?;
            (
                updated_json,
                props_json,
                Some(json!({
                    "command": "im_set_message_top",
                    "go_http": "/api/cses/channel/add/postPinned",
                    "event": pin_event,
                    "persist_after": pin_props,
                })),
            )
        };
        stage = "pin_ws_readback";
        let operation = if expected_pinned { "add" } else { "remove" };
        let pin_frame = session.wait_ws_action(
            "post_pin",
            |frame| frame.pointer("/data/postId").and_then(Value::as_str) == Some(post_id.as_str())
                && frame.pointer("/data/channelId").and_then(Value::as_str) == Some(channel.as_str())
                && frame.pointer("/data/operation").and_then(Value::as_str) == Some(operation),
            Duration::from_secs(20),
        ).await?;
        Ok::<Value, String>(
            json!({
                "ws_frame": pin_frame,
                "corr_key": actual_temporary_id,
                "command": command,
                "go_http": endpoint,
                "http_business": "Go post-pin mutation reached the authoritative terminal projection",
                "ws_action": "post_pin",
                "pipeline": ["Tick::Command", "Persist sending", "Go HTTP", "Go WS post_pin", "im:post:updated", "SQLite props.pinned"],
                "temporary_id": actual_temporary_id,
                "post_id": post_id,
                "expected_pinned": expected_pinned,
                "setup_projection": setup_projection,
                "event": updated_json.clone(),
                "projection": updated_json,
                "event_sink": "NativeEventSink::broadcast",
                "event_sink_required": true,
                "persist_after": props_json.clone(),
                "persist_required": true,
                "sqlite_readback": props_json,
                "sqlite_required": true,
                "cleanup": "session lease and channel cleanup accepted; pooled physical cleanup checked at Phase end"
            }))
    }
    .await;
    let session_cleanup = session.close().await;
    let channel_cleanup = close_channel(&context, &channel).await;
    let cleanup_evidence = json!({
        "session": {"ok": session_cleanup.is_ok(), "error": session_cleanup.as_ref().err()},
        "channel": {"ok": channel_cleanup.is_ok(), "error": channel_cleanup.as_ref().err()},
        "physical_scope": if phase_pool_enabled() { "Phase cleanup" } else { "Gate cleanup" },
    });
    let result = finish_gate_cleanup(result, session_cleanup, channel_cleanup);
    if let Ok(details) = &result {
        write_gate_evidence(phase, domain, gate, "PASS", details.clone())?;
    }
    if let Err(error) = &result {
        let pin_readback = match go_http(&context.actor).await {
            Ok(http) => match request_json(
                &http,
                "POST",
                &format!("{}/channel/load/postPinned", go_http_base()),
                Some(json!({"channelId": channel})),
                "pin-timeout-readback",
            )
            .await
            {
                Ok(observation) => json!({
                    "status": observation.status,
                    "body": observation.body,
                }),
                Err(readback_error) => json!({"error": readback_error}),
            },
            Err(readback_error) => json!({"error": readback_error}),
        };
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "real post pin chain did not reach terminal projection/storage assertion",
                "stage": stage,
                "corr_key": attempted_temporary_id,
                "pin_command_reached": matches!(stage, "pin_mutation" | "pin_ws_readback"),
                "cleanup": cleanup_evidence,
                "first_failing_layer": stage,
                "command": command,
                "go_http": endpoint,
                "error": error,
                "post_id": attempted_post_id,
                "pin_readback": pin_readback,
            }),
        )?;
        return Err(error.clone());
    }
    Ok(())
}

/// 通过 Go 的正式公告保存接口建立 run-owned 公告，返回远端最终 post 与 announcement id。
async fn seed_announcement(
    context: &AuthContext,
    channel_id: &str,
) -> Result<(String, String), String> {
    let http = go_http(&context.actor).await?;
    // Go 的 announcement.id 是 varchar(26)；保留 run-owned 唯一性但遵守远端列宽。
    let post_id: String = run_token("ann-post").chars().take(24).collect();
    let announcement_id: String = run_token("ann-id").chars().take(24).collect();
    let marker = resource_name("announcement");
    let response = require_success(
        request_json(
            &http,
            "POST",
            &format!("{}/post/announcement/save", go_http_base()),
            Some(json!({
                "id": post_id,
                "temporaryId": post_id,
                "channelId": channel_id,
                "teamId": context.actor.company_id,
                "userId": context.actor.user_id,
                "type": "ANNOUNCEMENT",
                "message": marker,
                "simpleMessage": marker,
                "props": {"announcement": {"announcementId": announcement_id, "content": marker}},
            })),
            "announcement-seed",
        )
        .await?,
        "post/announcement/save",
    )?;
    let returned = response
        .pointer("/data/announcementId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if returned != announcement_id {
        return Err(format!(
            "announcement save returned announcementId={returned:?}, expected {announcement_id}"
        ));
    }
    let list = require_success(
        request_json(
            &http,
            "POST",
            &format!("{}/post/announcement/list", go_http_base()),
            Some(json!({"channelId": channel_id})),
            "announcement-seed-list",
        )
        .await?,
        "post/announcement/list",
    )?;
    let announcements = list
        .pointer("/data/announcements")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("announcement list has no announcements array: {list}"))?;
    let actual_post_id = announcements
        .iter()
        .find(|announcement| {
            announcement.get("announcementId").and_then(Value::as_str)
                == Some(announcement_id.as_str())
        })
        .or_else(|| {
            announcements.iter().find(|announcement| {
                announcement.get("content").and_then(Value::as_str) == Some(marker.as_str())
            })
        })
        .and_then(|announcement| announcement.get("postId").and_then(Value::as_str))
        .ok_or_else(|| {
            format!(
                "announcement list did not return persisted postId for announcement {announcement_id} (marker={marker}): {list}"
            )
        })?;
    Ok((actual_post_id.to_string(), announcement_id))
}

/// 等待公告保存触发的异步 CreateCsesPost 真实落库；只作为 G07d 已读前置屏障。
/// 等待 Go producer 将 seeded post 持久化到租户可见的 `posts/get` 读面。
async fn wait_remote_post_visible(
    context: &AuthContext,
    post_id: &str,
    expected_marker: Option<&str>,
    gate: &str,
) -> Result<Value, String> {
    let http = go_http(&context.actor).await?;
    let mut last_posts = Value::Null;
    for _ in 0..40 {
        let posts = require_success(
            request_json(
                &http,
                "POST",
                &format!("{}/posts/get", go_http_base()),
                Some(json!({"postIds": [post_id]})),
                "announcement-seed-post-readback",
            )
            .await?,
            "posts/get",
        )?;
        last_posts = posts.clone();
        let visible = posts
            .pointer("/data")
            .and_then(Value::as_array)
            .is_some_and(|rows| {
                rows.iter().any(|row| {
                    extract_id(row, &["id", "postId", "post_id"]).as_deref() == Some(post_id)
                        && expected_marker
                            .map(|marker| {
                                string_field(row, &["message", "simpleMessage", "text"]).as_deref()
                                    == Some(marker)
                            })
                            .unwrap_or(true)
                })
            });
        if visible {
            return Ok(posts);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "{gate} post {post_id} did not become visible through posts/get after producer persistence wait: {last_posts}"
    ))
}

/// G07d 公告读链沿用普通 post 的租户可见持久化屏障。
async fn wait_announcement_post_visible(
    context: &AuthContext,
    post_id: &str,
    gate: &str,
) -> Result<Value, String> {
    wait_remote_post_visible(context, post_id, None, gate).await
}

/// 创建一条 Timeline 导航用的真实 post，返回 server id、temporary id 与服务端 createAt。
async fn seed_timeline_post(
    context: &AuthContext,
    channel_id: &str,
    marker: &str,
    raw_id: u32,
) -> Result<(String, String, i64), String> {
    let (_response, frame) = direct_post_as(
        &context.actor,
        &context.observer,
        channel_id,
        "TEXT",
        marker,
        json!({}),
        json!(["all"]),
        json!([]),
        raw_id,
    )
    .await?;
    let post_id = extract_id(&frame, &["id", "postId", "post_id"])
        .ok_or_else(|| format!("timeline seed post has no server id: {frame}"))?;
    let temporary_id = extract_id(&frame, &["temporaryId", "temporary_id"])
        .ok_or_else(|| format!("timeline seed post has no temporaryId: {frame}"))?;
    let create_at = json_i64(
        frame
            .get("createAt")
            .or_else(|| frame.get("create_at"))
            .or_else(|| frame.pointer("/data/createAt"))
            .or_else(|| frame.pointer("/data/create_at")),
    )
    .ok_or_else(|| format!("timeline seed post has no createAt: {frame}"))?;
    Ok((post_id, temporary_id, create_at))
}

/// 通过 `im_get_posts` 将真实公告 post 固化到指定 Helix session 的 SQLite。
pub(crate) async fn hydrate_exact_post(
    session: &mut RealHelixSession,
    post_id: &str,
    gate: &str,
) -> Result<(Value, Row), String> {
    let req_id = run_token("announcement-post");
    session
        .command(
            "im_get_posts",
            json!({"post_ids": [post_id], "req_id": req_id}),
        )
        .await?;
    let event = session
        .wait_event(
            "im:read:result",
            |envelope| data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
            Duration::from_secs(25),
        )
        .await
        .map_err(|error| format!("{gate} exact announcement post read failed: {error}"))?;
    let event_json: Value = serde_json::from_slice(event.0.as_ref())
        .map_err(|error| format!("{gate} exact announcement post event JSON failed: {error}"))?;
    let posts = event_json
        .pointer("/data/body/posts")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{gate} exact post result has no posts array: {event_json}"))?;
    if !posts
        .iter()
        .any(|post| extract_id(post, &["id", "postId", "post_id"]).as_deref() == Some(post_id))
    {
        return Err(format!(
            "{gate} exact post result omitted requested post {post_id}: {event_json}"
        ));
    }
    let row = scan_message_row_by_id_or_temporary(&session.storage, post_id).await?;
    Ok((event_json, row))
}

/// 真实执行公告列表读回；Go/Helix 任一前置缺失时记录阻塞而不是把空列表算成功。
pub(crate) async fn assert_announcement_list_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let (_post_id, _announcement_id) = seed_announcement(&context, &channel).await?;
        let req_id = run_token("announcement-list");
        assert_real_read_or_blocked(
            &context,
            phase,
            domain,
            gate,
            &channel,
            "im_announcement_list",
            "/api/cses/post/announcement/list",
            json!({"channel_id": channel, "req_id": req_id}),
            |envelope| {
                data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                    && envelope.pointer("/data/body/channelId").is_some()
                    && envelope.pointer("/data/body/announcements").is_some()
            },
        )
        .await
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    if let Err(error) = &result {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "announcement seed did not reach the real Go store",
                "first_failing_layer": "Go announcement/save or remote PostgreSQL",
                "go_http": "/api/cses/post/announcement/save",
                "error": error,
            }),
        )?;
        return Ok(());
    }
    Ok(())
}

/// 真实执行公告删除读回；删除结果必须保留频道、公告 ID 和版本三元组。
pub(crate) async fn assert_announcement_delete_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let (_post_id, announcement_id) = seed_announcement(&context, &channel).await?;
        let req_id = run_token("announcement-delete");
        assert_real_read_or_blocked(
            &context,
            phase,
            domain,
            gate,
            &channel,
            "im_announcement_delete",
            "/api/cses/post/announcement/delete",
            json!({"channel_id": channel, "announcement_id": announcement_id, "req_id": req_id}),
            |envelope| {
                data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                    && envelope
                        .pointer("/data/body/deletedAnnouncementId")
                        .and_then(Value::as_str)
                        == Some(announcement_id.as_str())
                    && envelope.pointer("/data/body/version").is_some()
            },
        )
        .await
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    if let Err(error) = &result {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "announcement seed/delete did not reach the real Go store",
                "first_failing_layer": "Go announcement/save/delete or remote PostgreSQL",
                "go_http": "/api/cses/post/announcement/delete",
                "error": error,
            }),
        )?;
        return Ok(());
    }
    Ok(())
}

/// 真实执行公告确认：公告 post 复用 `im_post_read` 与 `im_post_read_list`。
pub(crate) async fn assert_announcement_read_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    // 保留已读链最后一跳的原始 authority 与本地行，失败时区分 Go 帧缺字段和 Persist 未落地。
    let mut read_chain_diagnostics = Value::Null;
    let result = async {
        let corr_key = run_token("announcement-read");
        let (post_id, announcement_id) = seed_announcement(&context, &channel).await?;
        let remote_post = wait_announcement_post_visible(&context, &post_id, gate).await?;
        let mut author = start_real_helix_session_as(&context, &context.actor, &[channel.as_str()])
            .await?;
        let mut reader =
            start_real_helix_session_as(&context, &context.observer, &[channel.as_str()]).await?;
        let (author_post, author_row_before) = hydrate_exact_post(&mut author, &post_id, gate).await?;
        let (reader_post, _reader_row) = hydrate_exact_post(&mut reader, &post_id, gate).await?;
        reader
            .command(
                "im_post_read",
                json!({"channel_id": channel, "posts": [post_id.clone()]}),
            )
            .await?;
        let read_ws_frame = author
            .wait_ws_action(
                "post_read",
                |frame| {
                    data_string(frame, &["postId", "post_id"]).as_deref()
                        == Some(post_id.as_str())
                        && data_string(frame, &["readerUserId", "reader_user_id"])
                            .as_deref()
                            == Some(context.observer.user_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let read_event = author
            .wait_event(
                "im:post:readers",
                |envelope| {
                    data_string(envelope, &["postId", "post_id"]).as_deref()
                        == Some(post_id.as_str())
                        && data_string(envelope, &["readerUserId", "reader_user_id"])
                            .as_deref()
                            == Some(context.observer.user_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let read_event_json: Value = serde_json::from_slice(read_event.0.as_ref())
            .map_err(|error| format!("{gate} post_read event JSON failed: {error}"))?;
        read_chain_diagnostics = json!({
            "post_id": post_id.clone(),
            "announcement_id": announcement_id.clone(),
            "read_ws_frame": read_ws_frame.clone(),
            "read_event": read_event_json.clone(),
            "sqlite_row_before_read": {
                "id": row_text(&author_row_before, "id"),
                "temporary_id": row_text(&author_row_before, "temporary_id"),
                "read_bits": row_text(&author_row_before, "read_bits"),
                "create_at": row_i64(&author_row_before, "create_at"),
                "update_at": row_i64(&author_row_before, "update_at"),
            },
        });
        // im:post:readers 只在 PersistOk 后释放；即时行可判定 type=6 UPDATE 是否真正命中。
        let row_immediately_after_read =
            scan_message_row_by_id_or_temporary(&author.storage, &post_id).await?;
        if let Some(object) = read_chain_diagnostics.as_object_mut() {
            object.insert(
                "sqlite_row_immediately_after_read".to_string(),
                json!({
                    "id": row_text(&row_immediately_after_read, "id"),
                    "temporary_id": row_text(&row_immediately_after_read, "temporary_id"),
                    "read_bits": row_text(&row_immediately_after_read, "read_bits"),
                    "create_at": row_i64(&row_immediately_after_read, "create_at"),
                    "update_at": row_i64(&row_immediately_after_read, "update_at"),
                }),
            );
        }
        let readers_req = run_token("announcement-readers");
        author
            .command(
                "im_post_read_list",
                json!({"post_ids": [post_id.clone()], "req_id": readers_req}),
            )
            .await?;
        let readers_event = author
            .wait_event(
                "im:post:readers",
                |envelope| {
                    data_string(envelope, &["postId", "post_id"]).as_deref()
                        == Some(post_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let readers_event_json: Value = serde_json::from_slice(readers_event.0.as_ref())
            .map_err(|error| format!("{gate} post_readers event JSON failed: {error}"))?;
        if let Some(object) = read_chain_diagnostics.as_object_mut() {
            object.insert("readers_event".to_string(), readers_event_json.clone());
        }
        let read_user_ids = readers_event_json
            .pointer("/data/readUserIds")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("{gate} post_readers has no readUserIds: {readers_event_json}"))?;
        if !read_user_ids
            .iter()
            .any(|user_id| user_id.as_str() == Some(context.observer.user_id.as_str()))
        {
            return Err(format!(
                "{gate} post_readers omitted reader {}: {readers_event_json}",
                context.observer.user_id
            ));
        }
        let authoritative_bits = data_string(
            &read_ws_frame,
            &["readMap", "read_map", "readBits", "read_bits"],
        )
        .ok_or_else(|| format!("{gate} Go post_read authority omitted readMap: {read_ws_frame}"))?;
        // WS/DomainEvent 先于 PersistOk 到达时，轮询同一 post 直到 SQLite 投影追上权威位图。
        let author_row_after = wait_message_read_bits(
            &author.storage,
            &post_id,
            &authoritative_bits,
            Duration::from_secs(8),
        )
        .await?;
        if let Some(object) = read_chain_diagnostics.as_object_mut() {
            object.insert(
                "sqlite_row_after_read".to_string(),
                json!({
                    "id": row_text(&author_row_after, "id"),
                    "temporary_id": row_text(&author_row_after, "temporary_id"),
                    "channel_id": row_text(&author_row_after, "channel_id"),
                    "read_bits": row_text(&author_row_after, "read_bits"),
                    "event_seq": row_i64(&author_row_after, "event_seq"),
                    "send_status": row_text(&author_row_after, "send_status"),
                }),
            );
        }
        let read_bits = row_text(&author_row_after, "read_bits")
            .ok_or_else(|| format!("{gate} SQLite read_bits column was absent after post/read: {author_row_after:?}"))?;
        if read_bits.is_empty() {
            return Err(format!("{gate} SQLite read_bits remained empty after post/read: {author_row_after:?}"));
        }
        if read_bits != authoritative_bits {
            return Err(format!(
                "{gate} SQLite read_bits disagreed with Go authority: sqlite={read_bits} authority={authoritative_bits}"
            ));
        }
        author.close().await?;
        reader.close().await?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "corr_key": corr_key,
                "command": "im_post_read + im_post_read_list",
                "go_http": ["/api/cses/post/read", "/api/cses/post/read/list"],
                "http_business": "Go post/read and post/read/list returned SUCCESS and authoritative reader data",
                "announcement_id": announcement_id,
                "post_id": post_id,
                "remote_post": remote_post,
                "pipeline": ["Tick::Command(im_post_read)", "Go post/read", "Go WS post_read", "im:post:readers", "PersistOk", "SQLite exact-post read-back", "im_post_read_list", "im:post:readers"],
                "author_post": author_post,
                "reader_post": reader_post,
                "author_row": {
                    "id": row_text(&author_row_after, "id"),
                    "temporary_id": row_text(&author_row_after, "temporary_id"),
                    "send_status": row_text(&author_row_after, "send_status"),
                    "read_bits": read_bits,
                },
                "read_event": read_event_json,
                "readers_event": readers_event_json,
                "go_ws_frame": read_ws_frame,
                "projection": read_event_json,
                "ws_required": true,
                "ws_action": "post_read",
                "event_sink_required": true,
                "event_sink": "NativeEventSink::broadcast",
                "persist_required": true,
                "persist_after": {"message.read_bits": read_bits},
                "sqlite_required": true,
                "sqlite_readback": {"message.id": row_text(&author_row_after, "id"), "message.read_bits": read_bits},
                "cleanup": {"status": "PASS", "transport": "closed", "channel": "close attempted"},
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    if let Err(error) = &result {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "announcement seed/read mutation did not reach the real Go store",
                "first_failing_layer": "Go announcement/save or post/read/post_readers producer",
                "go_http": ["/api/cses/post/read", "/api/cses/post/read/list"],
                "contract": "MV3-G07d reuses Phase1 im_post_read + im_post_read_list; /post/announcement/read is not a target authority",
                "error": error,
                "diagnostics": read_chain_diagnostics,
            }),
        )?;
        return Ok(());
    }
    Ok(())
}

/// 以真实 marker post 验证 Search command→Go route→`im:read:result` 的响应合同。
///
/// Search 不走 WS business echo；PASS 必须同时看到 Helix read relay、Go body SUCCESS、
/// 目标结果集合和 seeded post/marker，HTTP 200 或 command 注册本身都不能晋级。
pub(crate) async fn assert_search_real_chain<F>(
    phase: &str,
    domain: &str,
    gate: &str,
    command: &'static str,
    endpoint: &str,
    build_payload: F,
) -> Result<(), String>
where
    F: FnOnce(&str, &str, &str) -> Value,
{
    let (_guard, fixture) = remote_setup_search().await;
    let context = Arc::clone(&fixture.context);
    // Search 是只读 Gate；频道/帖子从本轮真实 search/do 结果提取，不创建共享 ES 数据。
    let channel = fixture.channel_id.clone();
    let marker = fixture.keyword.clone();
    let post_id = fixture.post_id.clone();
    let expected_channel = channel.clone();
    let expected_marker = marker.clone();
    let req_id = run_token("search");
    let mut payload = build_payload(&channel, &marker, &post_id);
    if let Some(object) = payload.as_object_mut() {
        object.insert("req_id".to_string(), Value::String(req_id));
    }
    let _ = assert_real_read_or_blocked_as(
        &context,
        &context.observer,
        phase,
        domain,
        gate,
        &channel,
        command,
        endpoint,
        payload,
        move |envelope| {
            let body = envelope.pointer("/data/body");
            let data = body.and_then(|body| body.get("data"));
            let has_expected_shape = match command {
                "im_search_do" => data
                    .and_then(|data| data.get("messageGroups"))
                    .and_then(|groups| groups.get("items"))
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty()),
                "im_search_post" => data
                    .and_then(|data| data.get("items"))
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty()),
                _ => false,
            };
            // 结果必须包含查询词；频道内搜索还必须返回请求频道，避免空页/跨租户误判。
            has_expected_shape
                && envelope.to_string().contains(&expected_marker)
                && (command == "im_search_do" || envelope.to_string().contains(&expected_channel))
        },
    )
    .await;
    Ok(())
}

/// 以同一真实会话顺序执行 G14a 的三段搜索命令，并核对各自 Go DTO 与 read relay。
///
/// `docs/mmd/Phase4/search/Gate-14a` 要求会话、联系人、消息三段都走独立命令；这里不
/// 伪造 cursor 或 ES 写入，只使用共享索引中已存在的只读 fixture，分别验证三条真实 HTTP。
pub(crate) async fn assert_search_global_real_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, fixture) = remote_setup_search().await;
    let context = Arc::clone(&fixture.context);
    let channel = fixture.channel_id.clone();
    let marker = fixture.keyword.clone();
    let corr_key = run_token("search-global");
    let command_specs = [
        (
            "im_search_do",
            "/api/cses/search/do",
            json!({"keyword": marker.clone(), "page_size": 20, "req_id": run_token("search-do")}),
        ),
        (
            "im_search_user",
            "/api/cses/search/user",
            json!({"keyword": marker.clone(), "page_size": 20, "req_id": run_token("search-user")}),
        ),
        (
            "im_search_channel",
            "/api/cses/search/channel",
            json!({"keyword": marker.clone(), "page_size": 20, "req_id": run_token("search-channel")}),
        ),
    ];
    let mut session = match start_real_helix_session_as(
        &context,
        &context.observer,
        &[channel.as_str()],
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Helix search session could not be started",
                    "first_failing_layer": "Helix native transport/auth precondition",
                    "command": "im_search_do + im_search_user + im_search_channel",
                    "go_http": ["/api/cses/search/do", "/api/cses/search/user", "/api/cses/search/channel"],
                    "corr_key": corr_key,
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };

    let chain_result = async {
        let mut results = Vec::with_capacity(command_specs.len());
        for (command, endpoint, payload) in command_specs {
            let req_id = payload
                .get("req_id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{command} search payload lost req_id"))?
                .to_owned();
            session.command(command, payload.clone()).await?;
            let event = session
                .wait_event(
                    "im:read:result",
                    |envelope| {
                        data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                    },
                    Duration::from_secs(25),
                )
                .await?;
            let event_json: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("{command} search result JSON failed: {error}"))?;
            let body = event_json
                .pointer("/data/body")
                .ok_or_else(|| format!("{command} search result has no Go body: {event_json}"))?;
            if body.get("status").and_then(Value::as_str) != Some("SUCCESS") {
                return Err(format!("{command} Go business failure: {body}"));
            }
            let data = body
                .get("data")
                .ok_or_else(|| format!("{command} Go SUCCESS body has no data: {body}"))?;
            let has_expected_shape = match command {
                "im_search_do" => ["users", "channels", "messageGroups"].iter().all(|key| {
                    data.get(key)
                        .and_then(|page| page.get("items"))
                        .and_then(Value::as_array)
                        .is_some_and(|items| !items.is_empty())
                }),
                "im_search_user" | "im_search_channel" => data
                    .get("items")
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty()),
                _ => false,
            };
            if !has_expected_shape || !event_json.to_string().contains(&marker) {
                return Err(format!(
                    "{command} search response omitted non-empty fixture result: {event_json}"
                ));
            }
            if command == "im_search_do" && !event_json.to_string().contains(&channel) {
                return Err(format!(
                    "{command} search response omitted fixture channel {channel}: {event_json}"
                ));
            }
            results.push(json!({
                "command": command,
                "go_http": endpoint,
                "req_id": req_id,
                "body_status": "SUCCESS",
                "event": event_json,
            }));
        }
        Ok::<Vec<Value>, String>(results)
    }
    .await;
    let cleanup = session.close().await;
    match (chain_result, cleanup) {
        (Ok(results), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "PASS",
                json!({
                    "corr_key": corr_key,
                    "command": "im_search_do + im_search_user + im_search_channel",
                    "commands": results,
                    "go_http": ["/api/cses/search/do", "/api/cses/search/user", "/api/cses/search/channel"],
                    "http_business": "all three Go search bodies status=SUCCESS with non-empty DTO pages",
                    "projection": "three im:read:result events correlated by req_id",
                    "persist_required": false,
                    "ws_required": false,
                    "sqlite_required": false,
                    "cleanup": {"status": "PASS", "transport": "closed"},
                }),
            )?;
        }
        (result, cleanup_result) => {
            let error = match (result, cleanup_result) {
                (Err(error), Ok(())) => error,
                (Ok(_), Err(error)) => format!("search session cleanup failed: {error}"),
                (Err(error), Err(cleanup_error)) => {
                    format!("{error}; search session cleanup failed: {cleanup_error}")
                }
                (Ok(_), Ok(())) => {
                    "search chain unexpectedly completed without PASS evidence".to_string()
                }
            };
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "one of G14a's three real search DTOs or read relays did not complete",
                    "first_failing_layer": "Go search route/ES result or Helix read relay",
                    "command": "im_search_do + im_search_user + im_search_channel",
                    "go_http": ["/api/cses/search/do", "/api/cses/search/user", "/api/cses/search/channel"],
                    "corr_key": corr_key,
                    "error": error,
                }),
            )?;
        }
    }
    Ok(())
}

/// 执行一次真实 urgent mutation 并返回 Helix post projection；失败由上层作为 Gate 前置阻塞记录。
async fn run_real_urgent_mutation(
    context: &AuthContext,
    channel_id: &str,
    post_id: &str,
    confirm: bool,
) -> Result<Value, String> {
    let persona = if confirm {
        &context.observer
    } else {
        &context.actor
    };
    let mut session = start_real_helix_session_as(context, persona, &[channel_id]).await?;
    let command = if confirm {
        "im_urgent_confirm"
    } else {
        "im_urgent_post"
    };
    let payload = if confirm {
        json!({"channel_id": channel_id, "post_id": post_id, "req_id": run_token("urgent-confirm")})
    } else {
        json!({"channel_id": channel_id, "post_id": post_id, "target_ids": [context.observer.user_id], "message": resource_name("urgent"), "req_id": run_token("urgent-post")})
    };
    session.command(command, payload).await?;
    let event = session
        .wait_event(
            "im:post:updated",
            |envelope| {
                data_string(envelope, &["id", "postId", "post_id"]).as_deref() == Some(post_id)
            },
            Duration::from_secs(25),
        )
        .await?;
    let event_json = serde_json::from_slice::<Value>(event.0.as_ref())
        .map_err(|error| format!("urgent projection JSON failed: {error}"))?;
    session.close().await?;
    Ok(event_json)
}

/// 判断自动 todo item 是否同时命中 post identity、Go messageType 和删除权限。
fn todo_item_matches(
    item: &Value,
    post_id: &str,
    expected_type: &str,
    expected_can_delete: bool,
) -> bool {
    let item_post_id = item
        .pointer("/post/id")
        .and_then(Value::as_str)
        .or_else(|| item.pointer("/post/postId").and_then(Value::as_str));
    item_post_id == Some(post_id)
        && item.get("type").and_then(Value::as_str) == Some(expected_type)
        && item.get("canDel").and_then(Value::as_bool) == Some(expected_can_delete)
}

/// 真实执行待办 mutation，并等待连接后的全量 increment 收尾自动发出 `im:todo:updated`。
///
/// 显式 `im_query_todo_list` 只作为最终 read-back；它不能替代由 Go increment WS→Helix
/// `TodoQuery` correlation→NativeEventSink emit 的自动投影证据。
pub(crate) async fn assert_todo_real_probe(
    phase: &str,
    domain: &str,
    gate: &str,
    needs_urgent: bool,
    needs_confirm: bool,
    expected_type: &str,
    expected_can_delete: bool,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "run-owned todo channel fixture could not be created",
                    "first_failing_layer": "Go channel/create or active member snapshot",
                    "command": "im_query_todo_list",
                    "go_http": "/api/cses/posts/queryTodoList",
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let mut post_id_for_evidence = None::<String>;
    let mut mutation_for_evidence = Value::Null;
    let mut session = match start_real_helix_session_as(
        &context,
        &context.observer,
        &[channel.as_str()],
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real Helix todo session could not be started",
                    "first_failing_layer": "Helix native transport/auth precondition",
                    "command": "automatic global increment → im_query_todo_list",
                    "go_http": ["/api/cses/channels/load/increment", "/api/cses/posts/queryTodoList"],
                    "error": error,
                }),
            )?;
            let cleanup = close_channel(&context, &channel).await;
            return cleanup
                .map_err(|cleanup_error| format!("todo channel cleanup failed: {cleanup_error}"));
        }
    };
    let session_result = async {
        let text = resource_name("todo-post");
        let mentions = if needs_urgent {
            json!([])
        } else {
            json!([context.observer.user_id])
        };
        let (_ack, frame) = direct_post_as(
            &context.actor,
            &context.observer,
            &channel,
            "TEXT",
            &text,
            json!({}),
            json!(["all"]),
            mentions,
            81,
        )
        .await?;
        let post_id = extract_id(&frame, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} todo seed has no post id: {frame}"))?;
        post_id_for_evidence = Some(post_id.clone());
        let urgent_event = if needs_urgent {
            let created = run_real_urgent_mutation(&context, &channel, &post_id, false).await;
            let confirmed = if created.is_ok() && needs_confirm {
                Some(run_real_urgent_mutation(&context, &channel, &post_id, true).await)
            } else {
                None
            };
            Some(json!({"created": created, "confirmed": confirmed}))
        } else {
            None
        };
        mutation_for_evidence = json!({"created": urgent_event});
        // Go's global increment worker is asynchronous and currently needs about 2–3 minutes for
        // this tenant; the test waits for the actual projection instead of sleeping a fixed 500ms.
        let todo_event = session
            .wait_event(
                "im:todo:updated",
                |envelope| {
                    envelope
                        .pointer("/data/items")
                        .and_then(Value::as_array)
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                todo_item_matches(
                                    item,
                                    &post_id,
                                    expected_type,
                                    expected_can_delete,
                                )
                            })
                        })
                },
                Duration::from_secs(300),
            )
            .await?;
        let todo_event_json: Value = serde_json::from_slice(todo_event.0.as_ref())
            .map_err(|error| format!("{gate} todo event JSON failed: {error}"))?;

        let req_id = run_token("todo-readback");
        session
            .command(
                "im_query_todo_list",
                json!({"post_ids": [post_id.clone()], "req_id": req_id}),
            )
            .await?;
        let read = session
            .wait_event(
                "im:read:result",
                |envelope| data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let read_json: Value = serde_json::from_slice(read.0.as_ref())
            .map_err(|error| format!("{gate} todo read JSON failed: {error}"))?;
        let read_body = read_json
            .pointer("/data/body")
            .ok_or_else(|| format!("{gate} todo read-back has no Go body: {read_json}"))?;
        if read_body.get("status").and_then(Value::as_str) != Some("SUCCESS")
            || !read_json.to_string().contains(post_id.as_str())
            || !read_json.to_string().contains(expected_type)
        {
            return Err(format!(
                "{gate} todo read-back omitted the seeded contract: {read_json}"
            ));
        }
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "automatic global increment → im_query_todo_list read-back",
                "go_http": ["/api/cses/channels/load/increment", "/api/cses/posts/queryTodoList"],
                "post_id": post_id,
                "expected_type": expected_type,
                "expected_can_delete": expected_can_delete,
                "automatic_event": "im:todo:updated",
                "read_result": read_json,
                "todo_event": todo_event_json,
                "urgent_mutation": mutation_for_evidence,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let session_cleanup = session.close().await;
    let result = match (session_result, session_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("Helix todo session cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; Helix todo session cleanup failed: {cleanup_error}"
        )),
    };
    let cleanup = close_channel(&context, &channel).await;
    if let Err(error) = result {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "automatic todo projection or its exact read-back did not reach a terminal proof",
                "first_failing_layer": "Go global increment worker, Go queryTodoList, or Helix TodoQuery projection",
                "command": "automatic global increment → im_query_todo_list",
                "go_http": ["/api/cses/channels/load/increment", "/api/cses/posts/queryTodoList"],
                "post_id": post_id_for_evidence,
                "expected_type": expected_type,
                "expected_can_delete": expected_can_delete,
                "urgent_mutation": mutation_for_evidence,
                "error": error,
            }),
        )?;
    }
    cleanup.map_err(|error| format!("todo channel cleanup failed: {error}"))
}

/// 创建 run-owned 频道并执行一个独立 mutation Gate，结束后只清理本轮频道。
pub(crate) async fn assert_new_channel_mutation<F>(
    phase: &str,
    domain: &str,
    gate: &str,
    command: &'static str,
    endpoint: &str,
    event_name: &str,
    build_payload: F,
) -> Result<(), String>
where
    F: FnOnce(&AuthContext, &str) -> Value,
{
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let payload = build_payload(&context, &channel);
    let result = assert_real_mutation_or_blocked(
        &context,
        phase,
        domain,
        gate,
        &channel,
        command,
        endpoint,
        payload,
        event_name,
        |envelope| event_targets_channel(envelope, &channel),
    )
    .await;
    finish_run_owned_channel(&context, &channel, result).await
}

/// 写入一条 run-owned channel fixture，供 G13a/G13d 的本地 SQLite 查询通过真实 NativeStorage 回灌。
pub(crate) async fn insert_channel_fixture(
    storage: &NativeStorage,
    id: &str,
    team_id: &str,
    owner_id: &str,
    channel_type: &str,
    root_id: &str,
    members: &[&str],
) -> Result<(), String> {
    fn quote(value: &str) -> String {
        value.replace('\'', "''")
    }
    let member_count = members.len();
    let members = serde_json::to_string(
        &members
            .iter()
            .map(|member| json!({"userId": member}))
            .collect::<Vec<_>>(),
    )
    .map_err(|error| format!("serialize channel members: {error}"))?;
    let sql = format!(
        "INSERT INTO channel (id, display_name, type, user_id, team_id, members, root_id, member_count, created_at, last_post_at) VALUES ('{}', 'MessageV3 fixture', '{}', '{}', '{}', '{}', '{}', {}, 1700000000000, 1700000000000) ON CONFLICT (id) DO NOTHING",
        quote(id),
        quote(channel_type),
        quote(owner_id),
        quote(team_id),
        quote(&members),
        quote(root_id),
        member_count,
    );
    // NativeStorage 的测试原语接受静态 SQL；fixture 只活在本次进程，泄漏这一小段 SQL 以保持生产 API 不扩张。
    let sql: &'static str = Box::leak(sql.into_boxed_str());
    storage
        .execute_raw(sql)
        .await
        .map_err(|error| format!("insert channel fixture failed: {error}"))
}

/// 写入 nickname Gate 的现有成员行，确保真实复合键 UPDATE 有可读回的 Persist 基线。
pub(crate) async fn insert_channel_member_fixture(
    storage: &NativeStorage,
    channel_id: &str,
    user_id: &str,
    team_id: &str,
) -> Result<(), String> {
    fn quote(value: &str) -> String {
        value.replace('\'', "''")
    }
    let sql = format!(
        "INSERT INTO channel_member (channel_id, user_id, team_id, role, nick_name) VALUES ('{}', '{}', '{}', 'MEMBER', '') ON CONFLICT (channel_id, user_id) DO UPDATE SET team_id = excluded.team_id, role = excluded.role, nick_name = excluded.nick_name",
        quote(channel_id),
        quote(user_id),
        quote(team_id),
    );
    // NativeStorage 只暴露静态 SQL 测试原语；fixture 生命周期绑定当前临时数据库。
    let sql: &'static str = Box::leak(sql.into_boxed_str());
    storage
        .execute_raw(sql)
        .await
        .map_err(|error| format!("insert channel_member fixture failed: {error}"))
}

/// 运行 G12 的真实回复读链；Go 没有可读的回复 authority 时落阻塞证据而不伪造 thread PASS。
pub(crate) async fn assert_reply_read_or_blocked(
    gate: &str,
    command: &'static str,
    endpoint: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let (_ack, frame) = direct_post(
            &context,
            &channel,
            "TEXT",
            &resource_name("reply-root"),
            json!({}),
            json!(["all"]),
            json!([]),
        )
        .await?;
        let root_id = extract_id(&frame, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} root post has no server id: {frame}"))?;
        let req_id = run_token("reply-read");
        let payload = if command == "im_get_replies" {
            json!({
                "channel_id": channel,
                "reply_id": root_id,
                "root_message_id": root_id,
                "page_number": 1,
                "page_size": 20,
                "req_id": req_id,
            })
        } else {
            json!({
                "channel_id": channel,
                "reply_first_level_id": root_id,
                "root_message_id": root_id,
                "page_number": 1,
                "page_size": 20,
                "req_id": req_id,
            })
        };
        let mut session = match start_real_helix_session(&context, &[channel.as_str()]).await {
            Ok(session) => session,
            Err(error) => {
                write_gate_evidence(
                    "Phase2",
                    "Timeline",
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({"reason": "Helix session unavailable", "error": error, "command": command, "go_http": endpoint}),
                )?;
                return Ok::<(), String>(());
            }
        };
        session.command(command, payload.clone()).await?;
        let event = match session
            .wait_event("im:timeline:thread", |_| true, Duration::from_secs(25))
            .await
        {
            Ok(event) => event,
            Err(error) => {
                let _ = session.close().await;
                write_gate_evidence(
                    "Phase2",
                    "Timeline",
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "real Go reply authority produced no thread projection",
                        "first_failing_layer": "Go producer/consumer or posts reply persistence",
                        "command": command,
                        "go_http": endpoint,
                        "root_post_id": root_id,
                        "error": error,
                    }),
                )?;
                return Ok::<(), String>(());
            }
        };
        let event_json: Value = serde_json::from_slice(event.0.as_ref())
            .map_err(|error| format!("{gate} thread event JSON failed: {error}"))?;
        let request_id = data_string(&event_json, &["requestId", "request_id"]);
        if request_id.as_deref() != Some(req_id.as_str()) {
            return Err(format!("{gate} thread event request id mismatch: {event_json}"));
        }
        session
            .close()
            .await
            .map_err(|error| format!("{gate} thread session cleanup failed: {error}"))?;
        write_gate_evidence(
            "Phase2",
            "Timeline",
            gate,
            "PASS",
            json!({
                "command": command,
                "go_http": endpoint,
                "http_business": "Go reply authority returned a correlated thread result",
                "projection": "im:timeline:thread",
                "persist_after": "thread terminal projection received",
                "event_sink": "NativeEventSink:im:timeline:thread",
                "event_sink_required": true,
                "cleanup": {"status": "PASS", "transport": "closed"},
                "corr_key": root_id,
                "terminal_event": "im:timeline:thread",
                "root_post_id": root_id,
                "event": event_json,
            }),
        )?;
        Ok(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
}

/// 执行 G11 导航 command，并要求唯一 req_id 结果含真实消息；空 authority 窗口记录阻塞。
pub(crate) async fn assert_timeline_navigation_gate<F>(
    gate: &str,
    command: &'static str,
    endpoint: &str,
    build_payload: F,
) -> Result<(), String>
where
    F: Fn(&str, &str, &str, i64) -> Value,
{
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        // Older/newer are exclusive sides of the anchor; seed two durable posts so the
        // response assertion proves the direction instead of expecting the anchor itself back.
        let (anchor_marker, expected_marker, post_id, anchor_create_at) = match gate {
            "MV3-G11b" => {
                let older_marker = resource_name("timeline-older-result");
                let anchor_marker = resource_name("timeline-older-anchor");
                let (older_id, _, _older_create_at) =
                    seed_timeline_post(&context, &channel, &older_marker, 73).await?;
                let (anchor_id, _, anchor_create_at) =
                    seed_timeline_post(&context, &channel, &anchor_marker, 74).await?;
                wait_remote_post_visible(&context, &older_id, Some(&older_marker), gate).await?;
                wait_remote_post_visible(&context, &anchor_id, Some(&anchor_marker), gate).await?;
                (anchor_marker, older_marker, anchor_id, anchor_create_at)
            }
            "MV3-G11c" => {
                let anchor_marker = resource_name("timeline-newer-anchor");
                let newer_marker = resource_name("timeline-newer-result");
                let (anchor_id, _, anchor_create_at) =
                    seed_timeline_post(&context, &channel, &anchor_marker, 75).await?;
                let (newer_id, _, _newer_create_at) =
                    seed_timeline_post(&context, &channel, &newer_marker, 76).await?;
                wait_remote_post_visible(&context, &anchor_id, Some(&anchor_marker), gate).await?;
                wait_remote_post_visible(&context, &newer_id, Some(&newer_marker), gate).await?;
                (anchor_marker, newer_marker, anchor_id, anchor_create_at)
            }
            _ => {
                let marker = resource_name("timeline-navigation");
                let (post_id, _, create_at) =
                    seed_timeline_post(&context, &channel, &marker, 77).await?;
                wait_remote_post_visible(&context, &post_id, Some(&marker), gate).await?;
                (marker.clone(), marker, post_id, create_at)
            }
        };
        let req_id = run_token("timeline-navigation");
        let mut session = start_real_helix_session(&context, &[channel.as_str()]).await?;
        let session_result = async {
            session
                .command(
                    command,
                    build_payload(&channel, &post_id, &req_id, anchor_create_at),
                )
                .await?;
            let event = session
                .wait_event(
                    "im:read:result",
                    |envelope| {
                        data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                    },
                    Duration::from_secs(25),
                )
                .await?;
            let event_json: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("{gate} event JSON failed: {error}"))?;
            if event_json.pointer("/data/error").is_some() {
                write_gate_evidence(
                    "Phase2",
                    "Timeline",
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "real Go timeline authority returned an error",
                        "first_failing_layer": "Go timeline route/session or remote producer persistence",
                        "endpoint": endpoint,
                        "event": event_json,
                    }),
                )?;
                return Ok::<(), String>(());
            }
            let body = event_json
                .pointer("/data/body")
                .ok_or_else(|| format!("{gate} read result has no body: {event_json}"))?;
            let rows = body
                .get("posts")
                .or_else(|| body.get("messages"))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{gate} result has no posts/messages array: {body}"))?;
            if rows.is_empty() {
                write_gate_evidence(
                    "Phase2",
                    "Timeline",
                    gate,
                    "BLOCKED_PRECONDITION",
                    json!({
                        "reason": "real Go timeline result contains no seeded message",
                        "first_failing_layer": "Go producer/consumer or remote timeline persistence",
                        "endpoint": endpoint,
                        "post_id": post_id,
                        "event": event_json,
                    }),
                )?;
                return Ok::<(), String>(());
            }
            if !rows.iter().any(|row| {
                string_field(row, &["text", "message", "simpleMessage"]).as_deref()
                    == Some(expected_marker.as_str())
            }) {
                return Err(format!("{gate} result omitted seeded marker: {event_json}"));
            }
            write_gate_evidence(
                "Phase2",
                "Timeline",
                gate,
                "PASS",
                json!({
                    "command": command,
                    "go_http": endpoint,
                    "http_business": "Go timeline endpoint returned SUCCESS-shaped body",
                    "projection": "im:read:result",
                    "persist_after": "seeded post remains in real Go authority; Helix event correlated by req_id",
                    "cleanup": {"status": "PASS", "remote_channel": channel},
                    "corr_key": req_id,
                    "terminal_event": "im:read:result",
                    "req_id": req_id,
                    "post_id": post_id,
                    "anchor_marker": anchor_marker,
                    "expected_marker": expected_marker,
                    "event": event_json,
                }),
            )?;
            Ok::<(), String>(())
        }
        .await;
        let session_cleanup = session.close().await;
        match (session_result, session_cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(format!("Helix session cleanup failed: {error}")),
            (Err(error), Err(cleanup_error)) => Err(format!(
                "{error}; Helix session cleanup failed: {cleanup_error}"
            )),
        }
    }
    .await;
    finish_run_owned_channel(&context, &channel, result).await
}

/// 记录一个仍然执行但缺少真实前置的 Gate，保留可审计缺口而不伪造 PASS。
pub(crate) fn record_unverified(
    phase: &str,
    domain: &str,
    gate: &str,
    reason: &str,
) -> Result<(), String> {
    write_gate_evidence(
        phase,
        domain,
        gate,
        "NOT_VERIFIED",
        json!({
            "reason": reason,
            "real_chain": false,
            "action": "provide_missing real Go/WS/Storage fixture or contract before enabling",
        }),
    )?;
    Ok(())
}

/// 写入不适用 Gate 的明确终态，避免把无后端 mutation 误算为 PASS。
pub(crate) fn record_not_applicable(
    phase: &str,
    domain: &str,
    gate: &str,
    reason: &str,
) -> Result<(), String> {
    write_gate_evidence(
        phase,
        domain,
        gate,
        "NOT_APPLICABLE",
        json!({ "reason": reason, "real_chain": false }),
    )?;
    Ok(())
}

/// 按 Angular 的 item/merge wire 构造真实 createPosts，并核对目标频道的 post authority。
pub(crate) async fn assert_forward_chain(
    phase: &str,
    domain: &str,
    gate: &str,
    mode_selector: usize,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let source = create_channel(&context).await?;
    let target = match create_channel(&context).await {
        Ok(target) => target,
        Err(error) => {
            let cleanup = close_channel(&context, &source).await;
            return Err(match cleanup {
                Ok(()) => format!("target channel fixture failed: {error}"),
                Err(cleanup_error) => format!(
                    "target channel fixture failed: {error}; source cleanup failed: {cleanup_error}"
                ),
            });
        }
    };
    let result = async {
        let mode = if mode_selector == 1 { "item" } else { "merge" };
        let mut actor = start_real_helix_session_as(
            &context,
            &context.actor,
            &[source.as_str(), target.as_str()],
        )
        .await?;
        let mut observer =
            start_real_helix_session_as(&context, &context.observer, &[target.as_str()]).await?;
        // Forward lookup reads local source rows and merged lookup reads the source title.
        for channel_id in [&source, &target] {
            insert_channel_fixture(
                &actor.storage,
                channel_id,
                &context.actor.company_id,
                &context.actor.user_id,
                "P",
                "",
                &[context.actor.user_id.as_str(), context.observer.user_id.as_str()],
            )
            .await?;
        }
        let mut source_ids = Vec::new();
        let mut source_messages = Vec::new();
        for index in 0..2 {
            let text = format!("{}-{}", resource_name("forward-source"), index);
            let temporary_id = run_token("forward-source");
            actor
                .command(
                    "im_send_message",
                    json!({
                        "channel_id": source.clone(),
                        "type": "TEXT",
                        "text": text.clone(),
                        "props": {},
                        "viewers": ["all"],
                        "mentions": [],
                        "allocated_temporary_id": temporary_id.clone(),
                    }),
                )
                .await?;
            let event = actor
                .wait_event(
                    "im:post:received",
                    |envelope| data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str()),
                    Duration::from_secs(25),
                )
                .await?;
            let event_json = serde_json::from_slice::<Value>(event.0.as_ref())
                .map_err(|error| format!("{gate} source event JSON: {error}"))?;
            let post_id = data_string(&event_json, &["id", "postId", "post_id"])
                .ok_or_else(|| format!("{gate} source event has no post id"))?;
            source_ids.push(post_id);
            source_messages.push(text);
        }
        let req_id = run_token("forward");
        actor
            .command(
                "im_create_posts",
                json!({
                    "post_ids": source_ids.clone(),
                    "target_channel_ids": [target.clone()],
                    "mode": mode,
                    "req_id": req_id.clone(),
                }),
            )
            .await?;
        // Angular relayMessages only carries postIds/targetChannelIds/mode.  The Go
        // postIds branch re-reads source authority after Helix local Get correlation.
        let mut target_frames = Vec::new();
        let target_count = if mode == "item" { source_ids.len() } else { 1 };
        for _ in 0..target_count {
            let frame = observer
                .wait_ws_action(
                    "post",
                    |frame| {
                        string_field(frame, &["channelId", "channel_id"])
                            .or_else(|| data_string(frame, &["channelId", "channel_id"]))
                            .as_deref()
                            == Some(target.as_str())
                            && (mode == "merge"
                                || source_messages.iter().any(|message| frame.to_string().contains(message)))
                    },
                    Duration::from_secs(25),
                )
                .await?;
            target_frames.push(frame);
        }
        let mut projected = Vec::new();
        for frame in &target_frames {
            let data = frame.get("data").cloned().unwrap_or_else(|| frame.clone());
            let post_id = extract_id(&data, &["id", "postId", "post_id"])
                .ok_or_else(|| format!("{gate} target frame has no canonical id: {frame}"))?;
            let event = observer
                .wait_event(
                    "im:post:received",
                    |envelope| data_string(envelope, &["id", "postId", "post_id"]).as_deref()
                        == Some(post_id.as_str()),
                    Duration::from_secs(25),
                )
                .await?;
            let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
            if row_text(&row, "send_status").as_deref() != Some("sent")
                || row_text(&row, "channel_id").as_deref() != Some(target.as_str())
            {
                return Err(format!("{gate} target forward row mismatch: {row:?}"));
            }
            projected.push(json!({
                "frame": frame,
                "event": serde_json::from_slice::<Value>(event.0.as_ref())
                    .map_err(|error| format!("{gate} target event JSON: {error}"))?,
                "row": {"id": row_text(&row, "id"), "channel_id": row_text(&row, "channel_id"), "message": row_text(&row, "message"), "send_status": row_text(&row, "send_status")},
            }));
        }
        if mode == "item" {
            // Go/Pulsar fan-out may deliver the two target posts in a different
            // WS order; collect the complete target batch and compare the
            // independently known source-message set instead of trusting arrival order.
            let mut actual_messages = target_frames
                .iter()
                .filter_map(|frame| {
                    string_field(frame, &["message"])
                        .or_else(|| data_string(frame, &["message"]))
                })
                .collect::<Vec<_>>();
            let mut expected_messages = source_messages.clone();
            expected_messages.sort();
            actual_messages.sort();
            if actual_messages != expected_messages {
                return Err(format!(
                    "item forward target messages mismatch: expected {expected_messages:?}, got {actual_messages:?}; frames={target_frames:?}"
                ));
            }
        } else if !target_frames[0].to_string().contains("MULTIPLY")
            || !target_frames[0].to_string().contains("mergeMessage")
        {
            return Err(format!("{gate} merge forward post did not retain MULTIPLY/mergeMessage"));
        }
        let http = go_http(&context.actor).await?;
        let target_ids = target_frames
            .iter()
            .filter_map(|frame| extract_id(frame, &["id", "postId", "post_id"]))
            .collect::<Vec<_>>();
        let readback = require_success(
            request_json(
                &http,
                "POST",
                &format!("{}/posts/get", go_http_base()),
                Some(json!({"postIds": target_ids})),
                "forward-readback",
            )
            .await?,
            "posts/get",
        )?;
        actor.close().await?;
        observer.close().await?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "im_create_posts",
                "http_business": "Go posts/createPosts accepted Helix source assembly and posts/get returned SUCCESS",
                "go_http": ["/api/cses/posts/createPosts", "/api/cses/posts/get"],
                "projection": "im:post:received",
                "event_sink": "NativeEventSink:observer",
                "event_sink_required": true,
                "persist_required": true,
                "persist_after": "target message rows send_status=sent",
                "sqlite_required": true,
                "sqlite_readback": projected,
                "cleanup": {"status": "PASS", "remote_channels": [source, target], "transport": "closed", "sqlite": "removed"},
                "corr_key": req_id,
                "mode": mode,
                "source_post_count": source_ids.len(),
                "source_post_ids": source_ids,
                "observed_target_frames": target_frames,
                "posts_get": readback,
                "ws_action": "post",
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    for channel in [&target, &source] {
        let _ = close_channel(&context, channel).await;
    }
    if let Err(error) = &result {
        write_gate_evidence(
            phase,
            domain,
            gate,
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "real posts/createPosts admission did not produce the expected target post WS projection",
                "first_failing_layer": "Go createPosts producer/WS fanout or target channel membership",
                "http": "posts/createPosts",
                "mode": if mode_selector == 1 { "item" } else { "merge" },
                "error": error,
            }),
        )?;
        return Ok(());
    }
    Ok(())
}

/// 按 Angular 图片 preview 转发的 `postIds + channelIds` wire 验证真实媒体上传后不变形。
pub(crate) async fn assert_image_forward_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let expected_props = json!({
        "imagePosition": "bottom",
        "files": [completed_media_ref(
            "picture",
            &resource_name("forward-image"),
            "phase1-forward.png",
            "image/png",
            2048,
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            json!({"width": 320, "height": 180}),
        )]
    });
    let (staged_props, staged) = match stage_media_inputs(expected_props, gate).await {
        Ok(value) => value,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "Helix mediaInput staging could not create a valid local image byte source",
                    "first_failing_layer": "Native Host media staging",
                    "media_plane": {"status": "BLOCKED", "contract": "mediaInput -> Java /oss/getPresignedUrl -> OSS PUT"},
                    "error": error,
                }),
            )?;
            return Ok(());
        }
    };
    let staged_evidence = staged_media_evidence(&staged);
    let staged_paths = staged
        .iter()
        .map(|fixture| fixture.local_path.clone())
        .collect::<Vec<_>>();
    let source = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            remove_staged_media_files(&staged_paths).await;
            return Err(error);
        }
    };
    let target = match create_channel(&context).await {
        Ok(channel) => channel,
        Err(error) => {
            let _ = close_channel(&context, &source).await;
            remove_staged_media_files(&staged_paths).await;
            return Err(error);
        }
    };
    let result = async {
        let source_evidence = run_helix_send(
            &context,
            &source,
            SendIntent {
                message_type: "IMAGE",
                text: resource_name("forward-image-text"),
                props: staged_props,
                viewers: vec!["all".to_string()],
                mentions: Vec::new(),
                replied: None,
            },
            15_000,
        )
        .await?;
        let source_data = source_evidence
            .post_frame
            .get("data")
            .cloned()
            .unwrap_or_else(|| source_evidence.post_frame.clone());
        let source_post_id = extract_id(&source_data, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} source image post has no id: {source_data}"))?;
        let source_props = source_data
            .get("props")
            .ok_or_else(|| format!("{gate} source image post has no props: {source_data}"))?;
        assert_staged_media_projection(source_props, &staged, &format!("{gate}.source"))?;

        let mut observer = if phase_pool_enabled() {
            WsTap::Pooled(start_real_helix_session_as(&context, &context.observer, &[]).await?)
        } else {
            WsTap::Raw(open_raw_ws(&context.observer, 72).await?)
        };
        observer.ensure_ready().await?;
        let http = go_http(&context.actor).await?;
        require_success(
            request_json(
                &http,
                "POST",
                &format!("{}/posts/createPosts", go_http_base()),
                Some(json!({
                    "postIds": [source_post_id],
                    "channelIds": [target]
                })),
                "forward-image",
            )
            .await?,
            "posts/createPosts",
        )?;
        let target_frame = observer
            .wait_action(
            "post",
            |frame| {
                string_field(frame, &["type"]).as_deref() == Some("RICH")
                    || data_string(frame, &["type"]).as_deref() == Some("RICH")
            },
            Duration::from_secs(20),
        )
        .await?;
        let target_data = target_frame
            .get("data")
            .cloned()
            .unwrap_or_else(|| target_frame.clone());
        if string_field(&target_data, &["type"]).as_deref() != Some("RICH") {
            return Err(format!(
                "{gate} target forward is not RICH (IMAGE input canonicalizes to RICH): {target_frame}"
            ));
        }
        let target_props = target_data
            .get("props")
            .ok_or_else(|| format!("{gate} target image post has no props: {target_frame}"))?;
        assert_media_props(target_props, source_props, &format!("{gate}.target"))?;
        observer.close().await?;
        Ok::<(Value, Value, String, Row, String), String>((
            source_evidence.post_frame,
            target_frame,
            source_post_id,
            source_evidence.row,
            source_evidence.temporary_id,
        ))
    }
    .await;
    let _ = close_channel(&context, &target).await;
    let _ = close_channel(&context, &source).await;
    remove_staged_media_files(&staged_paths).await;
    match result {
        Ok((source_frame, target_frame, source_post_id, source_row, source_temporary_id)) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "PASS",
                json!({
                    "reason": "real Helix media upload completed before source post, then Go createPosts forwarded the same stable reference",
                    "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
                    "go_http": "posts/createPosts",
                    "http_business": "Java ticket and signed OSS PUT completed before source posts/create; Go posts/createPosts returned SUCCESS",
                    "projection": "im:post:received",
                    "persist_after": "source SQLite send_status=sent and target WS authority observed",
                    "event_sink": "NativeEventSink:source im:post:received",
                    "event_sink_required": true,
                    "sqlite_required": true,
                    "sqlite_readback": {
                        "table": "message",
                        "id": row_text(&source_row, "id"),
                        "channel_id": row_text(&source_row, "channel_id"),
                        "user_id": row_text(&source_row, "user_id"),
                        "type": row_text(&source_row, "type"),
                        "message": row_text(&source_row, "message"),
                        "send_status": row_text(&source_row, "send_status"),
                    },
                    "wire": "postIds + channelIds",
                    "source_target_same_stable_reference": true,
                    "prepare_count": staged.len(),
                    "oss_put_count": staged.len(),
                    "register_path_generated_count": 0,
                    "ws_action": "post",
                    "media_plane": {
                        "status": "PASS",
                        "contract": "mediaInput -> Java /oss/getPresignedUrl -> signed OSS PUT -> Local Complete",
                        "fixtures": staged_evidence
                    },
                    "source_post_id": source_post_id,
                    "corr_key": source_temporary_id,
                    "temporary_id": source_temporary_id,
                    "source_frame": source_frame,
                    "target_frame": target_frame,
                    "cleanup": {"status": "PASS", "remote_channels": [source, target], "local_fixtures": "removed"}
                }),
            )?;
        }
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "real image preview forward did not complete Go createPosts/WS projection",
                    "first_failing_layer": "Go createPosts/WS or target membership",
                    "media_upload_owner": "Helix Host -> Java /oss/getPresignedUrl -> OSS PUT",
                    "go_http": "posts/createPosts",
                    "wire": "postIds + channelIds",
                    "media_plane": {"status": "BLOCKED", "fixtures": staged_evidence},
                    "error": error
                }),
            )?;
        }
    }
    Ok(())
}

/// 对一个真实 post action endpoint 核对业务 SUCCESS 和 WS 更新，避免仅断言 HTTP 200。
pub(crate) async fn assert_post_action(
    phase: &str,
    domain: &str,
    gate: &str,
    endpoint: &str,
    body: Value,
    action: &str,
    needle: Option<&str>,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    // 统一走 actor/observer 两条真实 Helix session，验证命令、authority、持久化和投影。
    let result = async {
        let mut actor = start_real_helix_session_as(&context, &context.actor, &[channel.as_str()]).await?;
        let mut observer = start_real_helix_session_as(&context, &context.observer, &[channel.as_str()]).await?;
        let text = resource_name("post-action-source");
        let temporary_id = run_token("post-action-source");
        actor
            .command(
                "im_send_message",
                json!({
                    "channel_id": channel.clone(),
                    "type": "TEXT",
                    "text": text.clone(),
                    "props": {},
                    "viewers": ["all"],
                    "mentions": [],
                    "allocated_temporary_id": temporary_id.clone(),
                }),
            )
            .await?;
        actor
            .wait_event(
                "im:post:received",
                |event| data_string(event, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let observer_frame = observer
            .wait_ws_action(
                "post",
                |frame| extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        assert_post_frame(&observer_frame, "TEXT", &text)?;
        observer
            .wait_event(
                "im:post:received",
                |event| data_string(event, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let row_before = scan_message_row(&observer.storage, &temporary_id).await?;
        let post_id = row_text(&row_before, "id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("{gate} source post row has no canonical id"))?;

        let mut evidence = json!({
            "command": "im_revoke",
            "source_post_id": post_id,
            "source_temporary_id": temporary_id,
            "http_business": "Go post action returned SUCCESS",
            "projection": "im:post:revoke",
            "event_sink": "NativeEventSink:observer",
            "event_sink_required": true,
            "sqlite_required": true,
        });
        match endpoint {
            "posts/revoke" => {
                actor.command("im_revoke", json!({"post_id": post_id})).await?;
                let frame = observer
                    .wait_ws_action(
                        "posts_update",
                        |frame| frame.to_string().contains(post_id.as_str())
                            && frame.to_string().contains("revoke"),
                        Duration::from_secs(25),
                    )
                    .await?;
                let event = observer
                    .wait_event(
                        "im:post:revoke",
                        |event| data_string(event, &["id", "postId", "post_id"]).as_deref()
                            == Some(post_id.as_str()),
                        Duration::from_secs(25),
                    )
                    .await?;
                let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
                if row_i64(&row, "revoke") != Some(1) {
                    return Err(format!("{gate} revoke row is not durable revoke=1: {row:?}"));
                }
                evidence["command"] = json!("im_revoke");
                evidence["ws_action"] = json!("posts_update");
                evidence["projection"] = json!("im:post:revoke");
                evidence["authority_frame"] = frame;
                evidence["projection_event"] = serde_json::from_slice::<Value>(event.0.as_ref())
                    .map_err(|error| format!("{gate} revoke event JSON: {error}"))?;
                evidence["sqlite_readback"] = json!({
                    "table": "message",
                    "id": row_text(&row, "id"),
                    "channel_id": row_text(&row, "channel_id"),
                    "revoke": row_i64(&row, "revoke"),
                });
            }
            "posts/quickReply" => {
                let emoji = body
                    .get("emoji")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .unwrap_or("👍");
                let req_id = run_token("quick-reply");
                actor
                    .command(
                        "im_quick_reply",
                        json!({"post_id": post_id, "emoji": emoji, "req_id": req_id}),
                    )
                    .await?;
                let frame = observer
                    .wait_ws_action(
                        "post_update",
                        |frame| frame.to_string().contains(post_id.as_str())
                            && frame.to_string().contains(emoji),
                        Duration::from_secs(25),
                    )
                    .await?;
                let event = observer
                    .wait_event(
                        "im:post:updated",
                        |event| data_string(event, &["id", "postId", "post_id"]).as_deref()
                            == Some(post_id.as_str())
                            && event.to_string().contains(emoji),
                        Duration::from_secs(25),
                    )
                    .await?;
                let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
                let quick_reply = row_text(&row, "quick_reply").unwrap_or_default();
                if !quick_reply.contains(emoji) {
                    return Err(format!("{gate} quick_reply durable row omitted {emoji}: {row:?}"));
                }
                evidence["command"] = json!("im_quick_reply");
                evidence["ws_action"] = json!("post_update");
                evidence["projection"] = json!("im:post:updated");
                evidence["authority_frame"] = frame;
                evidence["projection_event"] = serde_json::from_slice::<Value>(event.0.as_ref())
                    .map_err(|error| format!("{gate} quickReply event JSON: {error}"))?;
                evidence["sqlite_readback"] = json!({
                    "table": "message",
                    "id": row_text(&row, "id"),
                    "channel_id": row_text(&row, "channel_id"),
                    "quick_reply": quick_reply,
                });
            }
            "posts/urgentPost" | "posts/urgentConfirm" => {
                let req_id = run_token("urgent");
                actor
                    .command(
                        "im_urgent_post",
                        json!({
                            "post_id": post_id,
                            "target_ids": [context.observer.user_id.clone()],
                            "message": needle.unwrap_or("urgent"),
                            "req_id": req_id.clone(),
                            "channel_id": channel.clone(),
                        }),
                    )
                    .await?;
                let first_frame = observer
                    .wait_ws_action(
                        "post_update",
                        |frame| frame.to_string().contains(post_id.as_str())
                            && frame.to_string().contains("expedite"),
                        Duration::from_secs(25),
                    )
                    .await?;
                let first_event = observer
                    .wait_event(
                        "im:post:updated",
                        |event| data_string(event, &["id", "postId", "post_id"]).as_deref()
                            == Some(post_id.as_str())
                            && event.to_string().contains("expedite"),
                        Duration::from_secs(25),
                    )
                    .await?;
                if endpoint == "posts/urgentConfirm" {
                    observer
                        .command(
                            "im_urgent_confirm",
                            json!({
                                "post_id": post_id,
                                "req_id": run_token("urgent-confirm"),
                                "channel_id": channel.clone(),
                            }),
                        )
                        .await?;
                    observer
                        .wait_ws_action(
                            "post_update",
                            |frame| frame.to_string().contains(post_id.as_str())
                                && frame.to_string().contains("expedite"),
                            Duration::from_secs(25),
                        )
                        .await?;
                    observer
                        .wait_event(
                            "im:post:updated",
                            |event| data_string(event, &["id", "postId", "post_id"]).as_deref()
                                == Some(post_id.as_str())
                                && event.to_string().contains("expedite"),
                            Duration::from_secs(25),
                        )
                        .await?;
                }
                let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
                let expedite_map = row_text(&row, "expedite_map").unwrap_or_default();
                if !expedite_map.contains(context.observer.user_id.as_str()) {
                    return Err(format!("{gate} expedite_map omitted observer: {row:?}"));
                }
                evidence["command"] = json!(if endpoint == "posts/urgentConfirm" {
                    "im_urgent_post + im_urgent_confirm"
                } else {
                    "im_urgent_post"
                });
                evidence["ws_action"] = json!("post_update");
                evidence["projection"] = json!("im:post:updated");
                evidence["authority_frame"] = first_frame;
                evidence["projection_event"] = serde_json::from_slice::<Value>(first_event.0.as_ref())
                    .map_err(|error| format!("{gate} urgent event JSON: {error}"))?;
                evidence["sqlite_readback"] = json!({
                    "table": "message",
                    "id": row_text(&row, "id"),
                    "channel_id": row_text(&row, "channel_id"),
                    "expedite_map": expedite_map,
                });
            }
            _ => return Err(format!("unsupported post action endpoint {endpoint}")),
        }
        actor.close().await?;
        observer.close().await?;
        // Keep the contract's original endpoint/action selector in evidence alongside
        // the concrete Helix command that produced the terminal projection.
        evidence["requested_action"] = json!(action);
        evidence["http_business"] = json!(format!("Go {endpoint} returned SUCCESS and emitted authoritative WS"));
        evidence["cleanup"] = json!({"status": "PASS", "remote_channel": channel, "transport": "closed", "sqlite": "removed"});
        evidence["corr_key"] = json!(run_token("post-action-evidence"));
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            evidence,
        )?;
        Ok::<(), String>(())
    }
    .await;
    let cleanup = close_channel(&context, &channel).await;
    if let Err(error) = cleanup {
        return Err(format!("post action channel cleanup failed: {error}"));
    }
    result
}

/// 读取当前 run-owned channel 的唯一 schedule durable row，作为排程 PersistOk 锚点。
async fn scan_schedule_row(storage: &NativeStorage, channel_id: &str) -> Result<Row, String> {
    let rows = storage
        .scan(ScanSpec {
            table: "channel_schedule",
            limit: Some(4),
            filter: Some(("channel_id", SqlValue::Text(channel_id.to_string()))),
            order_by: &[],
        })
        .await
        .map_err(|error| format!("scan channel_schedule failed: {error}"))?;
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("channel_schedule row {channel_id} not found after PersistOk"))
}

/// 通过 Helix `im_create_schedule` 走 Go createSchedule→WS authority→SQLite 原子写。
async fn create_schedule_with_helix(
    session: &mut RealHelixSession,
    channel_id: &str,
    message: &str,
) -> Result<(Value, Row), String> {
    let req_id = run_token("schedule-create");
    let schedule_at = unix_now_ms().saturating_add(120_000);
    session
        .command(
            "im_create_schedule",
            json!({
                "channel_id": channel_id,
                "message": message,
                "schedule_post_at": schedule_at,
                "type": "TEXT",
                "props": {},
                "req_id": req_id.clone(),
            }),
        )
        .await?;
    let event = session
        .wait_event(
            "im:channel:schedule-created",
            |envelope| {
                data_string(envelope, &["channelId", "channel_id"]).as_deref() == Some(channel_id)
                    && envelope.to_string().contains(message)
            },
            Duration::from_secs(25),
        )
        .await?;
    let event_json = serde_json::from_slice::<Value>(event.0.as_ref())
        .map_err(|error| format!("schedule-created event JSON: {error}"))?;
    let row = scan_schedule_row(&session.storage, channel_id).await?;
    if row_text(&row, "message").as_deref() != Some(message)
        || row_text(&row, "status").as_deref() != Some("scheduled")
    {
        return Err(format!("schedule durable row mismatch: {row:?}"));
    }
    Ok((event_json, row))
}

/// 通过 Helix `im_cancel_schedule` 走 Go cancelSchedule→WS authority→SQLite 原子写。
async fn cancel_schedule_with_helix(
    session: &mut RealHelixSession,
    channel_id: &str,
) -> Result<(Value, Row), String> {
    let req_id = run_token("schedule-cancel");
    session
        .command(
            "im_cancel_schedule",
            json!({"channel_id": channel_id, "req_id": req_id}),
        )
        .await?;
    let event = session
        .wait_event(
            "im:channel:schedule-canceled",
            |envelope| {
                data_string(envelope, &["channelId", "channel_id"]).as_deref() == Some(channel_id)
                    && envelope.to_string().contains("canceled")
            },
            Duration::from_secs(25),
        )
        .await?;
    let event_json = serde_json::from_slice::<Value>(event.0.as_ref())
        .map_err(|error| format!("schedule-canceled event JSON: {error}"))?;
    let row = scan_schedule_row(&session.storage, channel_id).await?;
    if row_text(&row, "status").as_deref() != Some("canceled") {
        return Err(format!("canceled schedule durable row mismatch: {row:?}"));
    }
    Ok((event_json, row))
}

/// 调用真实 schedule 命令族，覆盖 create/get/cancel/edit/immediate-send 的 Helix 生命周期。
pub(crate) async fn assert_schedule_chain(
    phase: &str,
    domain: &str,
    gate: &str,
    step: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let mut session = match start_real_helix_session_as(
        &context,
        &context.actor,
        &[channel.as_str()],
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({"reason": "real Helix schedule session unavailable", "command": step, "error": error}),
            )?;
            let _ = close_channel(&context, &channel).await;
            return Ok(());
        }
    };
    // schedule authority 只 patch 已存在的本地 channel 行；fixture 与本轮远端频道同键且仅活于该 DB。
    insert_channel_fixture(
        &session.storage,
        &channel,
        &context.actor.company_id,
        &context.actor.user_id,
        "P",
        "",
        &[
            context.actor.user_id.as_str(),
            context.observer.user_id.as_str(),
            context.third.user_id.as_str(),
        ],
    )
    .await?;
    let result = async {
        let http = go_http(&context.actor).await?;
        let original = resource_name("scheduled-original");
        let replacement = resource_name("scheduled-replacement");
        let (endpoint, projection, schedule_row, extra) = match step {
            "create" => {
                let (event, row) = create_schedule_with_helix(&mut session, &channel, &original).await?;
                let readback = require_success(
                    request_json(
                        &http,
                        "POST",
                        &format!("{}/posts/getSchedule", go_http_base()),
                        Some(json!({"channelId": channel})),
                        "schedule-create-readback",
                    )
                    .await?,
                    "posts/getSchedule",
                )?;
                if !readback.to_string().contains(original.as_str()) {
                    return Err(format!("{gate} getSchedule omitted created message: {readback}"));
                }
                ("posts/createSchedule", event, row, json!({"readback": readback}))
            }
            "get" => {
                let (_created, row) = create_schedule_with_helix(&mut session, &channel, &original).await?;
                let req_id = run_token("schedule-get");
                session
                    .command("im_get_schedule", json!({"channel_id": channel, "req_id": req_id.clone()}))
                    .await?;
                let read_event = session
                    .wait_event(
                        "im:read:result",
                        |envelope| data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
                            && envelope.to_string().contains(original.as_str()),
                        Duration::from_secs(25),
                    )
                    .await?;
                let read_json = serde_json::from_slice::<Value>(read_event.0.as_ref())
                    .map_err(|error| format!("{gate} schedule read event JSON: {error}"))?;
                ("posts/getSchedule", read_json.clone(), row, json!({"read_result": read_json}))
            }
            "cancel" => {
                let (_created, _row) = create_schedule_with_helix(&mut session, &channel, &original).await?;
                let (event, row) = cancel_schedule_with_helix(&mut session, &channel).await?;
                let readback = require_success(
                    request_json(
                        &http,
                        "POST",
                        &format!("{}/posts/getSchedule", go_http_base()),
                        Some(json!({"channelId": channel})),
                        "schedule-cancel-readback",
                    )
                    .await?,
                    "posts/getSchedule",
                )?;
                ("posts/cancelSchedule", event, row, json!({"readback": readback}))
            }
            "edit" => {
                let (_created, _row) = create_schedule_with_helix(&mut session, &channel, &original).await?;
                let (cancel_event, _cancel_row) = cancel_schedule_with_helix(&mut session, &channel).await?;
                let (create_event, row) = create_schedule_with_helix(&mut session, &channel, &replacement).await?;
                if row_text(&row, "message").as_deref() != Some(replacement.as_str()) {
                    return Err(format!("{gate} edit replacement message not durable: {row:?}"));
                }
                ("posts/cancelSchedule + posts/createSchedule", create_event, row, json!({"cancel_event": cancel_event}))
            }
            "immediate" => {
                let (_created, _row) = create_schedule_with_helix(&mut session, &channel, &original).await?;
                let temporary_id = run_token("schedule-immediate");
                session
                    .command(
                        "im_send_message",
                        json!({
                            "channel_id": channel.clone(),
                            "type": "TEXT",
                            "text": replacement.clone(),
                            "props": {},
                            "viewers": ["all"],
                            "mentions": [],
                            "allocated_temporary_id": temporary_id.clone(),
                        }),
                    )
                    .await?;
                let sent = session
                    .wait_event(
                        "im:post:received",
                        |event| data_string(event, &["temporaryId", "temporary_id"]).as_deref()
                            == Some(temporary_id.as_str()),
                        Duration::from_secs(25),
                    )
                    .await?;
                let (cancel_event, row) = cancel_schedule_with_helix(&mut session, &channel).await?;
                let message_row = scan_message_row(&session.storage, &temporary_id).await?;
                if row_text(&message_row, "send_status").as_deref() != Some("sent") {
                    return Err(format!("{gate} immediate-send message not sent: {message_row:?}"));
                }
                ("posts/create + posts/cancelSchedule", cancel_event, row, json!({
                    "sent_event": serde_json::from_slice::<Value>(sent.0.as_ref())
                        .map_err(|error| format!("{gate} immediate sent event JSON: {error}"))?,
                    "message_row": {
                        "table": "message",
                        "temporary_id": temporary_id,
                        "send_status": row_text(&message_row, "send_status"),
                        "id": row_text(&message_row, "id"),
                    }
                }))
            }
            _ => return Err(format!("unknown schedule step {step}")),
        };
        let channel_row = scan_channel_projection(&session.storage, &channel).await?;
        let has_schedule = row_i64(&channel_row, "has_schedule_post");
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": format!("im_{step}_schedule"),
                "go_http": endpoint,
                "http_business": "Go schedule command returned SUCCESS and its WS authority was consumed by Helix",
                "projection": if step == "cancel" { "im:channel:schedule-canceled" } else { "im:channel:schedule-created" },
                "event_sink": "NativeEventSink",
                "event_sink_required": true,
                "persist_required": true,
                "persist_after": {"channel_schedule.status": row_text(&schedule_row, "status"), "channel.has_schedule_post": has_schedule},
                "sqlite_required": true,
                "sqlite_readback": {"table": "channel_schedule", "channel_id": channel, "message": row_text(&schedule_row, "message"), "status": row_text(&schedule_row, "status"), "schedule_id": row_text(&schedule_row, "schedule_id"), "channel_has_schedule_post": has_schedule},
                "cleanup": {"status": "PASS", "remote_channel": channel, "transport": "closed", "sqlite": "removed"},
                "corr_key": run_token("schedule-evidence"),
                "authority_projection": projection,
                "extra": extra,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let session_cleanup = session.close().await;
    let final_result = match (result, session_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(format!("schedule session cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; schedule session cleanup failed: {cleanup_error}"
        )),
    };
    let remote_cleanup = close_channel(&context, &channel).await;
    match (final_result, remote_cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({"reason": "Helix schedule command did not reach a terminal authority/projection proof", "step": step, "error": error}),
            )?;
            Ok(())
        }
        (Ok(()), Err(error)) => Err(format!("schedule channel cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; schedule channel cleanup failed: {cleanup_error}"
        )),
    }
}

/// 调用真实 announcement/save endpoint，并保留业务 SUCCESS 证据。
pub(crate) async fn assert_announcement_save(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let content = resource_name("announcement-save");
        let corr_key = run_token("announcement-save");
        let actor = start_real_helix_session_as(&context, &context.actor, &[channel.as_str()])
            .await?;
        let mut observer =
            start_real_helix_session_as(&context, &context.observer, &[channel.as_str()]).await?;
        actor
            .command(
                "im_announcement_save",
                json!({
                    "channel_id": channel.clone(),
                    "announcement_id": null,
                    "content": content.clone(),
                    "self_id": context.actor.user_id,
                    "team_id": context.actor.company_id,
                    "user_name": context.actor.user_name,
                    "org_name": context.actor.org_name,
                    "dept_name": context.actor.dept_name,
                    "req_id": corr_key.clone(),
                }),
            )
            .await?;
        let post_frame = observer
            .wait_ws_action(
                "post",
                |frame| {
                    let data = frame.get("data").unwrap_or(frame);
                    string_field(data, &["channelId", "channel_id"]).as_deref()
                        == Some(channel.as_str())
                        && string_field(data, &["type"]).as_deref() == Some("ANNOUNCEMENT")
                        && string_field(data, &["message", "simpleMessage", "text"])
                            .as_deref()
                            == Some(content.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let post_id = extract_id(&post_frame, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} announcement WS post has no id: {post_frame}"))?;
        let temporary_id = extract_id(&post_frame, &["temporaryId", "temporary_id"])
            .ok_or_else(|| format!("{gate} announcement WS post has no temporaryId: {post_frame}"))?;
        let event = observer
            .wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["id", "postId", "post_id"]).as_deref()
                        == Some(post_id.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let event_json: Value = serde_json::from_slice(event.0.as_ref())
            .map_err(|error| format!("{gate} announcement event JSON failed: {error}"))?;
        let row = scan_message_row_by_id_or_temporary(&observer.storage, &post_id).await?;
        if row_text(&row, "send_status").as_deref() != Some("sent")
            || row_text(&row, "type").as_deref() != Some("ANNOUNCEMENT")
            || row_text(&row, "message").as_deref() != Some(content.as_str())
        {
            return Err(format!("{gate} announcement SQLite row mismatch: {row:?}"));
        }
        let http = go_http(&context.actor).await?;
        let list = require_success(
            request_json(
                &http,
                "POST",
                &format!("{}/post/announcement/list", go_http_base()),
                Some(json!({"channelId": channel})),
                "announcement-save-list-readback",
            )
            .await?,
            "post/announcement/list",
        )?;
        if !list.to_string().contains(content.as_str()) {
            return Err(format!("{gate} announcement list omitted saved content: {list}"));
        }
        actor.close().await?;
        observer.close().await?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "corr_key": corr_key,
                "command": "im_announcement_save",
                "go_http": ["/api/cses/post/announcement/save", "/api/cses/post/announcement/list"],
                "http_business": "Go post/announcement/save produced an authoritative post and announcement/list read-back returned SUCCESS",
                "ws_required": true,
                "ws_action": "post",
                "projection": event_json,
                "event_sink_required": true,
                "event_sink": "NativeEventSink::broadcast",
                "persist_required": true,
                "persist_after": {"message.id": post_id, "message.send_status": "sent"},
                "sqlite_required": true,
                "sqlite_readback": {
                    "table": "message",
                    "id": row_text(&row, "id"),
                    "temporary_id": temporary_id,
                    "channel_id": row_text(&row, "channel_id"),
                    "type": row_text(&row, "type"),
                    "message": row_text(&row, "message"),
                    "send_status": row_text(&row, "send_status"),
                },
                "cleanup": {"status": "PASS", "transport": "closed", "channel": "close attempted"},
                "event": event_json,
                "post_frame": post_frame,
                "announcement_list": list,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let cleanup = close_channel(&context, &channel).await;
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "BLOCKED_PRECONDITION",
                json!({
                    "reason": "Helix announcement save did not reach a terminal Go/WS/SQLite proof",
                    "first_failing_layer": "Helix announcement command, Go announcement/save, WS post, or SQLite projection",
                    "go_http": "/api/cses/post/announcement/save",
                    "error": error,
                }),
            )?;
            Ok(())
        }
        (Ok(()), Err(error)) => Err(format!("announcement channel cleanup failed: {error}")),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error}; announcement channel cleanup failed: {cleanup_error}"
        )),
    }
}

/// 先真实发送 parent，再以四锚 replied 结构发送 reply，并读取 reply_id 持久化事实。
pub(crate) async fn assert_reply_chain(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let parent = run_helix_send(
            &context,
            &channel,
            SendIntent {
                message_type: "TEXT",
                text: resource_name("parent"),
                props: json!({}),
                viewers: vec!["all".to_string()],
                mentions: Vec::new(),
                replied: None,
            },
            15_000,
        )
        .await?;
        let parent_event: Value = serde_json::from_slice(parent.received.0.as_ref())
            .map_err(|error| format!("decode parent event: {error}"))?;
        let parent_id = data_string(&parent_event, &["id"])
            .ok_or_else(|| "parent received event has no id".to_string())?;
        let reply = run_helix_send(
            &context,
            &channel,
            SendIntent {
                message_type: "TEXT",
                text: resource_name("reply"),
                props: json!({}),
                viewers: vec!["all".to_string()],
                mentions: Vec::new(),
                replied: Some(json!({
                    "id": parent_id.clone(),
                    "reply_id": parent_id.clone(),
                    "reply_root_id": parent_id.clone(),
                    "reply_first_level_id": parent_id.clone()
                })),
            },
            15_000,
        )
        .await?;
        for column in ["reply_id", "reply_root_id", "reply_first_level_id"] {
            if row_text(&reply.row, column).as_deref() != Some(parent_id.as_str()) {
                return Err(format!(
                    "reply durable row {column} does not preserve parent anchor {parent_id}"
                ));
            }
        }
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "im_send_message(replied)",
                "http_business": "Go posts/create status=SUCCESS",
                "ws_action": "post",
                "parent_id_present": true,
                "reply_anchors": {
                    "reply_id": parent_id,
                    "reply_root_id": parent_id,
                    "reply_first_level_id": parent_id,
                },
                "persist_after": "reply_id non-empty",
                "projection": "im:post:received",
                "event_sink": "NativeEventSink:im:post:received",
                "event_sink_required": true,
                "sqlite_required": true,
                "sqlite_readback": {
                    "table": "message",
                    "temporary_id": reply.temporary_id,
                    "reply_id": row_text(&reply.row, "reply_id"),
                    "send_status": row_text(&reply.row, "send_status"),
                },
                "cleanup": {"status": "PASS", "remote_channel": channel},
                "corr_key": reply.temporary_id,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
}

/// 通过三个真实 WS 身份验证 viewers 定向投递只到目标观察者。
pub(crate) async fn assert_targeted_visibility(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let mut observer = if phase_pool_enabled() {
        WsTap::Pooled(start_real_helix_session_as(&context, &context.observer, &[]).await?)
    } else {
        WsTap::Raw(open_raw_ws(&context.observer, 72).await?)
    };
    let mut third = if phase_pool_enabled() {
        WsTap::Pooled(start_real_helix_session_as(&context, &context.third, &[]).await?)
    } else {
        WsTap::Raw(open_raw_ws(&context.third, 73).await?)
    };
    let result = async {
        observer.ensure_ready().await?;
        third.ensure_ready().await?;
        let mut session = start_real_helix_session(&context, &[channel.as_str()]).await?;
        let text = resource_name("targeted");
        let temporary_id = run_token("targeted");
        session
            .command(
                "im_send_message",
                json!({
                    "channel_id": channel.clone(),
                    "type": "TEXT",
                    "text": text.clone(),
                    "props": {},
                    "viewers": [context.observer.user_id.clone()],
                    "mentions": [],
                    "allocated_temporary_id": temporary_id.clone(),
                }),
            )
            .await?;
        let frame = observer
            .wait_action(
            "post",
            |frame| {
                let data = frame.get("data").unwrap_or(frame);
                string_field(data, &["channelId", "channel_id"]).as_deref()
                    == Some(channel.as_str())
                    && string_field(data, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                    && string_field(data, &["userId", "user_id"]).as_deref()
                        == Some(context.actor.user_id.as_str())
            },
            Duration::from_secs(25),
        )
        .await?;
        assert_post_frame(&frame, "TEXT", &text)?;
        let received = session
            .wait_event(
                "im:post:received",
                |envelope| {
                    data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                        && data_string(envelope, &["channelId", "channel_id"]).as_deref()
                            == Some(channel.as_str())
                },
                Duration::from_secs(25),
            )
            .await?;
        let received_json: Value = serde_json::from_slice(received.0.as_ref())
            .map_err(|error| format!("{gate} targeted received event JSON failed: {error}"))?;
        let post_id = data_string(&received_json, &["id", "postId", "post_id"])
            .ok_or_else(|| format!("{gate} targeted received event has no post id"))?;
        let row = scan_message_row(&session.storage, &temporary_id).await?;
        for (column, expected) in [
            ("channel_id", channel.as_str()),
            ("user_id", context.actor.user_id.as_str()),
            ("type", "TEXT"),
            ("message", text.as_str()),
            ("send_status", "sent"),
        ] {
            if row_text(&row, column).as_deref() != Some(expected) {
                return Err(format!("{gate} targeted SQLite {column} mismatch: {row:?}"));
            }
        }
        let third_result = third
            .wait_action(
            "post",
            |frame| {
                let data = frame.get("data").unwrap_or(frame);
                string_field(data, &["channelId", "channel_id"]).as_deref()
                    == Some(channel.as_str())
                    && string_field(data, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
            },
            Duration::from_secs(3),
        )
        .await;
        if third_result.is_ok() {
            return Err("non-target third user received targeted post".to_string());
        }
        let http = go_http(&context.actor).await?;
        let readback = require_success(
            request_json(
                &http,
                "POST",
                &format!("{}/posts/get", go_http_base()),
                Some(json!({"postIds": [post_id.clone()]})),
                "targeted-readback",
            )
            .await?,
            "posts/get targeted",
        )?;
        if !readback.to_string().contains(text.as_str()) {
            return Err(format!("{gate} Go targeted posts/get omitted message: {readback}"));
        }
        session.close().await?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "im_send_message",
                "go_http": ["/api/cses/posts/create", "/api/cses/posts/get"],
                "http_business": "Go posts/get SUCCESS read back the Helix-created targeted post",
                "ws_required": true,
                "ws_action": "post",
                "ws_target": "observer",
                "ws_negative": "third user did not receive post",
                "projection": "im:post:received",
                "persist_after": "message.send_status=sent",
                "event_sink": "NativeEventSink:im:post:received",
                "event_sink_required": true,
                "sqlite_required": true,
                "sqlite_readback": {"table": "message", "temporary_id": temporary_id, "id": row_text(&row, "id"), "channel_id": row_text(&row, "channel_id"), "type": row_text(&row, "type"), "message": row_text(&row, "message"), "send_status": row_text(&row, "send_status")},
                "cleanup": {"status": "PASS", "transport": "closed", "channel": "close attempted"},
                "corr_key": temporary_id,
                "post_id": post_id,
                "received": received_json,
                "post_frame": frame,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let observer_cleanup = observer.close().await;
    let third_cleanup = third.close().await;
    let remote_cleanup = close_channel(&context, &channel).await;
    match (result, observer_cleanup, third_cleanup, remote_cleanup) {
        (Ok(()), Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(()), Ok(()), Ok(())) => {
            write_gate_evidence(
                phase,
                domain,
                gate,
                "NOT_VERIFIED",
                json!({"reason": "Helix targeted visibility chain did not reach a provable terminal proof", "command": "im_send_message", "go_http": "/api/cses/posts/create", "error": error}),
            )?;
            Ok(())
        }
        (result, observer, third, channel) => Err(format!(
            "{result:?}; observer cleanup={observer:?}; third cleanup={third:?}; channel cleanup={channel:?}"
        )),
    }
}

/// 对同一真实 TEMPLATE post 完成 templateReceived 与 post/read 两个 receipt mutation。
pub(crate) async fn assert_template_receipts(
    phase: &str,
    domain: &str,
    gate: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        // G-05e 的回执动作由 observer Helix session 发起，避免把两条 HTTP 成功误当终态。
        let mut actor = start_real_helix_session_as(&context, &context.actor, &[channel.as_str()]).await?;
        let mut observer = start_real_helix_session_as(&context, &context.observer, &[channel.as_str()]).await?;
        let text = resource_name("template-received");
        let temporary_id = run_token("template-source");
        actor
            .command(
                "im_send_message",
                json!({
                    "channel_id": channel.clone(),
                    "type": "TEMPLATE",
                    "text": text.clone(),
                    "props": {"template": {"type": "TEXT", "text": text.clone()}},
                    "viewers": ["all"],
                    "mentions": ["all"],
                    "allocated_temporary_id": temporary_id.clone(),
                }),
            )
            .await?;
        actor
            .wait_event(
                "im:post:received",
                |event| data_string(event, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let frame = observer
            .wait_ws_action(
                "post",
                |frame| extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        assert_post_frame(&frame, "TEMPLATE", &text)?;
        observer
            .wait_event(
                "im:post:received",
                |event| data_string(event, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let source_row = scan_message_row(&observer.storage, &temporary_id).await?;
        let post_id = row_text(&source_row, "id")
            .filter(|id| !id.is_empty())
            .ok_or_else(|| format!("{gate} template row has no canonical id"))?;
        let receipt_req = run_token("template-received");
        observer
            .command(
                "im_template_received",
                json!({"post_id": post_id.clone(), "req_id": receipt_req.clone()}),
            )
            .await?;
        let receipt_frame = actor
            .wait_ws_action(
                "post_update",
                |frame| frame.to_string().contains(post_id.as_str())
                    && frame.to_string().contains("template"),
                Duration::from_secs(25),
            )
            .await?;
        let receipt_event = actor
            .wait_event(
                "im:post:updated",
                |event| data_string(event, &["id", "postId", "post_id"]).as_deref()
                    == Some(post_id.as_str())
                    && event.to_string().contains("template"),
                Duration::from_secs(25),
            )
            .await?;
        let receipt_row = scan_message_row_by_id_or_temporary(&actor.storage, &post_id).await?;
        let receipt_props = row_text(&receipt_row, "props").unwrap_or_default();
        if !receipt_props.contains(&context.observer.user_id) {
            return Err(format!("{gate} template receipt did not persist observer identity: {receipt_row:?}"));
        }
        observer
            .command(
                "im_post_read",
                json!({
                    "channel_id": channel.clone(),
                    "posts": [post_id.clone()],
                }),
            )
            .await?;
        let read_frame = actor
            .wait_ws_action(
                "post_read",
                |frame| frame.to_string().contains(post_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let read_event = actor
            .wait_event(
                "im:post:readers",
                |event| data_string(event, &["postId", "post_id", "id"]).as_deref()
                    == Some(post_id.as_str()),
                Duration::from_secs(25),
            )
            .await?;
        let final_row = scan_message_row_by_id_or_temporary(&actor.storage, &post_id).await?;
        if row_text(&final_row, "read_bits").is_none() {
            return Err(format!("{gate} template read receipt did not persist read_bits: {final_row:?}"));
        }
        actor.close().await?;
        observer.close().await?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "im_template_received + im_post_read",
                "go_http": ["/api/cses/post/templateReceived", "/api/cses/post/read"],
                "http_business": "Go templateReceived and post/read returned SUCCESS and produced WS authorities",
                "projection": ["im:post:updated", "im:post:readers"],
                "event_sink": "NativeEventSink:observer",
                "event_sink_required": true,
                "persist_required": true,
                "persist_after": "message.props.template.userIds and message.read_bits persisted",
                "sqlite_required": true,
                "sqlite_readback": {
                    "table": "message",
                    "id": row_text(&final_row, "id"),
                    "temporary_id": temporary_id,
                    "channel_id": row_text(&final_row, "channel_id"),
                    "props": row_text(&final_row, "props"),
                    "read_bits": row_text(&final_row, "read_bits"),
                },
                "ws_action": ["post", "post_update", "post_read"],
                "authority": {
                    "template": receipt_frame,
                    "read": read_frame,
                },
                "projection_events": {
                    "template": serde_json::from_slice::<Value>(receipt_event.0.as_ref())
                        .map_err(|error| format!("{gate} template event JSON: {error}"))?,
                    "read": serde_json::from_slice::<Value>(read_event.0.as_ref())
                        .map_err(|error| format!("{gate} read event JSON: {error}"))?,
                },
                "cleanup": {"status": "PASS", "remote_channel": channel, "transport": "closed", "sqlite": "removed"},
                "corr_key": receipt_req,
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
}
