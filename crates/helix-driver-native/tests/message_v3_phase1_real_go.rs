//! MessageV3 Phase1 的 Rust→真实 Go 8066 集成测试。
//!
//! 这不是 fake server、fixture 回放或 Node 编排器：测试通过 native driver 的真实
//! `NativeHttp`、`NativeTransport`、`NativeStorage` 和 `NativeEventSink`，把 Helix
//! `ExecutionShell` 接到本机 Go 服务，并在同一轮核对 HTTP、WS、SQLite 与 emit。
//!
//! 运行方式：
//!
//! ```text
//! cargo test -p helix-driver-native --test message_v3_phase1_real_go -- --nocapture --test-threads=1
//! ```
//!
//! 默认不读取环境变量。测试会自动寻找现有 `cses-client/.wdio/credentials.local.json`，
//! 登录三个共享租户账号；Go 8066、登录服务或对象存储不可达时显式失败为
//! `BLOCKED_PRECONDITION/NOT_VERIFIED`，不会把 health 200、HTTP admission 或 fake WS 当成 PASS。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpRequest, Row, ScanSpec, SqlValue, TransportId};
use helix_core::ports::{HttpRequester, Storage};
use helix_core::tick::{AppCommand, Tick};
use helix_core::ExecutionShell;
use helix_driver_host::{HostNetworkConfig, NoopMetricSink};
use helix_driver_native::{
    bus_event_name, run_engine_loop_with_transports, to_bus_envelope, EngineConfig,
    NativeEventSink, NativeHttp, NativeStorage, NativeTransport, SharedFileUploader,
    TransportTable,
};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Digest;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, OnceCell};
use tokio::time::{timeout, Duration};

const GO_HTTP_BASE: &str = "http://127.0.0.1:8066/api/cses";
const GO_WS_URL: &str = "ws://127.0.0.1:8066/api/v4/websocket";
const MAIN_API_BASE: &str = "http://cses-server-paas25-test.cses.svc.cluster.local:3399";
const GO_HEALTH_URL: &str = "http://127.0.0.1:8066/api/cses/health";

/// 返回登录服务地址；默认保持集群域名，宿主机端口转发时允许测试显式覆盖。
fn main_api_base() -> String {
    std::env::var("HELIX_MAIN_API_BASE").unwrap_or_else(|_| MAIN_API_BASE.to_string())
}
const CREDENTIALS_RELATIVE: &str = "angular/visual/cses-client/.wdio/credentials.local.json";

static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static AUTH_CONTEXT: OnceCell<Result<Arc<AuthContext>, String>> = OnceCell::const_new();
static REMOTE_LOCK: OnceCell<Mutex<()>> = OnceCell::const_new();

#[derive(Debug, Deserialize)]
struct CredentialFile {
    accounts: Vec<CredentialAccount>,
    #[serde(default, alias = "defaultIndex")]
    default_index: usize,
}

#[derive(Debug, Deserialize)]
struct CredentialAccount {
    mobile: Option<String>,
    password: String,
}

#[derive(Clone, Debug)]
struct Persona {
    user_id: String,
    cookie_id: String,
    company_id: String,
    user_name: String,
    org_name: String,
    dept_name: String,
    companies: Vec<String>,
}

struct AuthContext {
    actor: Persona,
    observer: Persona,
    third: Persona,
}

#[derive(Debug)]
struct HttpObservation {
    status: u16,
    body: Value,
}

struct RawWs {
    transport: Arc<NativeTransport>,
    ticks: mpsc::Receiver<Tick>,
}

#[derive(Clone)]
struct SendIntent {
    message_type: &'static str,
    text: String,
    props: Value,
    viewers: Vec<String>,
    mentions: Vec<String>,
    replied: Option<Value>,
}

struct SendEvidence {
    temporary_id: String,
    sending: DomainEventBytes,
    received: DomainEventBytes,
    row: Row,
    bus_envelope: Value,
}

/// 生成不含账号/令牌的本轮 correlation 头和 run-owned 资源前缀。
fn run_token(prefix: &str) -> String {
    let seq = RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("hx-p1-{prefix}-{}-{seq}", std::process::id())
}

/// 返回当前测试使用的唯一远端资源前缀；只允许清理带该前缀的资源。
fn resource_name(domain: &str) -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("hx-p1-{domain}-{}-{millis}", std::process::id())
}

/// 在单进程内串行化远端业务写，避免测试并行时互相消费 WS 广播或重用频道。
async fn remote_setup() -> (tokio::sync::MutexGuard<'static, ()>, Arc<AuthContext>) {
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

/// 在固定候选路径中寻找 Angular 现有凭据，不从环境变量或用户输入读取 secret。
fn discover_credentials_path() -> Result<PathBuf, String> {
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

/// 创建一个没有身份头的 HTTP client，用于 Go health 或主登录接口。
async fn anonymous_http(base: &str) -> Result<NativeHttp, String> {
    NativeHttp::with_config(HostNetworkConfig::new(base, GO_WS_URL))
        .map_err(|error| format!("NativeHttp init failed for {base}: {error}"))
}

/// 发送 JSON 请求并保留业务信封；HTTP 200 不在此处等价于业务 SUCCESS。
async fn request_json(
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
    let response = http
        .request(request)
        .await
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
fn require_success(observation: HttpObservation, operation: &str) -> Result<Value, String> {
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
fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Some(text.clone()),
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => None,
    })
}

/// 从登录响应提取某租户的展示字段和所有可用 companyId。
fn persona_from_login(value: &Value, role: &'static str, mobile: &str) -> Result<Persona, String> {
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
async fn login_account(account: &CredentialAccount, role: &'static str) -> Result<Persona, String> {
    let mobile = account
        .mobile
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "credential account has no mobile; skipping unusable account".to_string())?;
    let main_api = main_api_base();
    let http = anonymous_http(&main_api).await?;
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
        &format!("{main_api}/User/login"),
        Some(json!({ "userName": mobile, "password": account.password })),
        "login",
    )
    .await?;
    if observation.status != 200 {
        return Err(format!("login HTTP status {}", observation.status));
    }
    persona_from_login(&observation.body, role, mobile)
}

/// 自动从现有账号池找出同一 companyId 下的 actor/observer/third 三个身份。
async fn discover_auth_context() -> Result<AuthContext, String> {
    let credentials_path = discover_credentials_path()?;
    let content = std::fs::read_to_string(&credentials_path)
        .map_err(|error| format!("read credentials file failed: {error}"))?;
    let credentials: CredentialFile = serde_json::from_str(&content)
        .or_else(|map_error| {
            serde_json::from_str::<Vec<CredentialAccount>>(&content)
                .map(|accounts| CredentialFile {
                    accounts,
                    default_index: 0,
                })
                .map_err(|_| map_error)
        })
        .map_err(|error| format!("parse credentials file failed: {error}"))?;
    if credentials.accounts.len() < 3 {
        return Err("credentials account pool must contain at least three accounts".to_string());
    }

    let health = anonymous_http(GO_HTTP_BASE).await?;
    let health_response = request_json(&health, "GET", GO_HEALTH_URL, None, "health").await?;
    if health_response.status != 200 {
        return Err(format!(
            "Go 8066 health returned HTTP {}",
            health_response.status
        ));
    }

    let mut order = Vec::with_capacity(credentials.accounts.len());
    order.push(
        credentials
            .default_index
            .min(credentials.accounts.len() - 1),
    );
    let selected = order.clone();
    order.extend((0..credentials.accounts.len()).filter(|index| !selected.contains(index)));
    let mut logged_in = Vec::new();
    for index in order {
        let role = match logged_in.len() {
            0 => "actor",
            1 => "observer",
            _ => "third",
        };
        if let Ok(persona) = login_account(&credentials.accounts[index], role).await {
            logged_in.push(persona);
        }
        if logged_in.len() >= 3 {
            let actor = &logged_in[0];
            if let Some(company_id) = actor.companies.iter().find(|company| {
                logged_in[1].companies.contains(company) && logged_in[2].companies.contains(company)
            }) {
                let mut actor = actor.clone();
                let mut observer = logged_in[1].clone();
                let mut third = logged_in[2].clone();
                actor.company_id = company_id.clone();
                observer.company_id = company_id.clone();
                third.company_id = company_id.clone();
                return Ok(AuthContext {
                    actor,
                    observer,
                    third,
                });
            }
        }
    }
    Err("could not find three logged-in accounts sharing one companyId".to_string())
}

/// 构造带 Go 身份头的 native HTTP client；HTTP actor 与 body userId 必须保持一致。
async fn go_http(persona: &Persona) -> Result<NativeHttp, String> {
    let http = NativeHttp::with_config(HostNetworkConfig::new(GO_HTTP_BASE, GO_WS_URL))
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
fn ws_headers(persona: &Persona) -> Vec<(String, String)> {
    vec![
        ("Cookieid".to_string(), persona.cookie_id.clone()),
        ("userId".to_string(), persona.user_id.clone()),
        ("companyId".to_string(), persona.company_id.clone()),
        ("teamId".to_string(), persona.company_id.clone()),
        ("deviceId".to_string(), "helix-phase1-rust".to_string()),
    ]
}

/// 打开真实 Go WS，先完成 upgrade/activation，再由调用方消费 Connected 与 hello。
async fn open_raw_ws(persona: &Persona, raw_id: u32) -> Result<RawWs, String> {
    let (tick_tx, ticks) = mpsc::channel(256);
    let mut transport =
        NativeTransport::new(GO_WS_URL, TransportId::from_raw(raw_id), Some(tick_tx))
            .with_handshake_headers(ws_headers(persona));
    let activation = transport
        .connect()
        .await
        .map_err(|error| format!("real Go WS connect failed: {error}"))?;
    activation
        .activate()
        .await
        .map_err(|error| format!("real Go WS activation failed: {error}"))?;
    Ok(RawWs {
        transport: Arc::new(transport),
        ticks,
    })
}

/// 等待指定 action 的真实 Go WS frame；Connected/噪音帧被消费但不会被当业务成功。
async fn wait_ws_action<F>(
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
            let tick = ws
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
            if frame.get("action").and_then(Value::as_str) == Some(action) && predicate(&frame) {
                return Ok(frame);
            }
        }
    })
    .await
    .map_err(|_| format!("timed out waiting real Go WS action '{action}'"))?
}

/// 只等待一次 hello，作为 WS authenticated session 的业务前置证据。
async fn wait_ws_hello(ws: &mut RawWs) -> Result<Value, String> {
    wait_ws_action(ws, "hello", |_| true, Duration::from_secs(10)).await
}

/// 等待指定领域事件，并按 data/temporaryId 做精确匹配。
async fn wait_domain_event<F>(
    receiver: &mut broadcast::Receiver<DomainEventBytes>,
    event_name: &str,
    predicate: F,
    timeout_duration: Duration,
) -> Result<DomainEventBytes, String>
where
    F: Fn(&Value) -> bool,
{
    timeout(timeout_duration, async {
        loop {
            let event = receiver
                .recv()
                .await
                .map_err(|error| format!("event sink receive error: {error}"))?;
            let envelope: Value = serde_json::from_slice(event.0.as_ref())
                .map_err(|error| format!("invalid Helix DomainEvent JSON: {error}"))?;
            if envelope.get("event").and_then(Value::as_str) == Some(event_name)
                && predicate(&envelope)
            {
                return Ok(event);
            }
        }
    })
    .await
    .map_err(|_| format!("timed out waiting Helix event '{event_name}'"))?
}

/// 从 event envelope 的 data 中取一个字符串字段，兼容 snake/camel wire。
fn data_string(envelope: &Value, keys: &[&str]) -> Option<String> {
    let data = envelope.get("data")?;
    string_field(data, keys)
}

/// 从 Go post/channel 响应中提取 canonical id，拒绝用 message 或随机字段冒充 id。
fn extract_id(value: &Value, keys: &[&str]) -> Option<String> {
    if let Some(id) = string_field(value, keys) {
        return Some(id);
    }
    match value {
        Value::Object(object) => object.values().find_map(|child| extract_id(child, keys)),
        Value::Array(items) => items.iter().find_map(|child| extract_id(child, keys)),
        _ => None,
    }
}

/// 以 canonical channel/create body 创建 run-owned 私有群聊。
async fn create_channel(context: &AuthContext) -> Result<String, String> {
    let http = go_http(&context.actor).await?;
    let display_name = resource_name("channel");
    let body = json!({
        "type": "P",
        "userIds": [context.observer.user_id, context.third.user_id],
        "displayName": display_name,
        "forceCreate": true
    });
    let observation = request_json(
        &http,
        "POST",
        &format!("{GO_HTTP_BASE}/channel/create"),
        Some(body),
        "channel-create",
    )
    .await?;
    let response = require_success(observation, "channel/create")?;
    extract_id(&response, &["id", "channelId", "channel_id"])
        .ok_or_else(|| "channel/create SUCCESS response has no canonical channel id".to_string())
}

/// 只关闭本测试刚创建的频道；不执行全库 truncate/flush。
async fn close_channel(context: &AuthContext, channel_id: &str) -> Result<(), String> {
    let http = go_http(&context.actor).await?;
    let observation = request_json(
        &http,
        "POST",
        &format!("{GO_HTTP_BASE}/channel/close"),
        Some(json!({ "channelId": channel_id })),
        "channel-close",
    )
    .await?;
    require_success(observation, "channel/close").map(|_| ())
}

/// 在真实 Go 上直接发送一条 post，返回 HTTP admission 和 observer 的 outbound post frame。
async fn direct_post(
    context: &AuthContext,
    channel_id: &str,
    message_type: &str,
    text: &str,
    props: Value,
    viewers: Value,
    mentions: Value,
) -> Result<(Value, Value), String> {
    let mut observer = open_raw_ws(&context.observer, 10).await?;
    wait_ws_hello(&mut observer).await?;
    let temporary_id = run_token("post");
    let http = go_http(&context.actor).await?;
    let body = json!({
        "channelId": channel_id,
        "userId": context.actor.user_id,
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
        &format!("{GO_HTTP_BASE}/posts/create"),
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
    observer
        .transport
        .close()
        .await
        .map_err(|error| format!("close observer WS: {error}"))?;
    Ok((response, post))
}

/// 初始化真实 IM SQLite schema；只对本测试新建数据库执行 CREATE/migration。
async fn apply_im_schema(storage: &NativeStorage) -> Result<(), String> {
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
async fn open_test_storage() -> Result<(NativeStorage, PathBuf), String> {
    let path = std::env::temp_dir().join(format!("{}.db", resource_name("sqlite")));
    let url = format!("sqlite:{}?mode=rwc", path.display());
    let storage = NativeStorage::open(&url)
        .await
        .map_err(|error| format!("open test SQLite failed: {error}"))?;
    apply_im_schema(&storage).await?;
    Ok((storage, path))
}

/// 删除本测试自己的 SQLite 文件，不触碰共享客户端数据库。
fn remove_test_storage(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// 扫描一个 temporary_id 的 durable message row，作为 Persist/WS 后的行级 readback。
async fn scan_message_row(storage: &NativeStorage, temporary_id: &str) -> Result<Row, String> {
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

/// 读取 row 的文本列，避免把 Debug 输出当成 storage 证据。
fn row_text(row: &Row, column: &str) -> Option<String> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Text(text)) if name == column => Some(text.clone()),
            _ => None,
        })
}

/// 按真实 engine 装配一次完整 send，返回 HTTP→WS→SQLite→emit 的逐跳证据。
async fn run_helix_send(
    context: &AuthContext,
    channel_id: &str,
    intent: SendIntent,
    send_timeout_ms: u64,
) -> Result<SendEvidence, String> {
    let mut observer = open_raw_ws(&context.observer, 20).await?;
    wait_ws_hello(&mut observer).await?;
    let (storage, db_path) = open_test_storage().await?;
    let (event_sink, mut events) = NativeEventSink::new();
    let http = go_http(&context.actor).await?;

    let mut config = ImConfig::default();
    config.ws_url = GO_WS_URL.to_string();
    config.api_base_url = GO_HTTP_BASE.to_string();
    config.default_api_base_url = main_api_base();
    config.auth_user_id = context.actor.user_id.clone();
    config.company_id = context.actor.company_id.clone();
    config.user_name = context.actor.user_name.clone();
    config.org_name = context.actor.org_name.clone();
    config.dept_name = context.actor.dept_name.clone();
    config.ping_interval_ms = 60_000;
    config.send_timeout_ms = send_timeout_ms;

    let mut module = ImModule::new(config);
    let channel = helix_im::state::ChannelId::from_str(channel_id)
        .ok_or_else(|| format!("Go channel id is not a canonical Helix ChannelId: {channel_id}"))?;
    module.register_channel(channel, 0);
    let mut shell = ExecutionShell::new();
    shell.register(module);

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let mut actor_transport =
        NativeTransport::new(GO_WS_URL, TransportId::from_raw(21), Some(tick_tx.clone()))
            .with_handshake_headers(ws_headers(&context.actor));
    let activation = actor_transport
        .connect()
        .await
        .map_err(|error| format!("Helix actor WS connect failed: {error}"))?;
    let actor_transport = Arc::new(actor_transport);
    let mut transports = TransportTable::new();
    transports.insert(TransportId::from_raw(21), Arc::clone(&actor_transport));

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let config = EngineConfig::new(
        storage.clone(),
        http,
        SharedFileUploader::default(),
        event_sink,
        8,
    )
    .with_metric_sink(Arc::new(NoopMetricSink));
    let engine = tokio::spawn(run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx.clone(),
        config,
        shutdown_rx,
        transports,
    ));
    activation
        .activate()
        .await
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

    let optimistic_row = scan_message_row(&storage, &temporary_id).await?;
    if row_text(&optimistic_row, "send_status").as_deref() != Some("sending") {
        return Err(format!(
            "message {temporary_id} did not persist optimistic sending row: {:?}",
            row_text(&optimistic_row, "send_status")
        ));
    }

    let _post_frame = wait_ws_action(
        &mut observer,
        "post",
        |frame| {
            extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                == Some(temporary_id.as_str())
        },
        Duration::from_secs(25),
    )
    .await?;
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
    let row = scan_message_row(&storage, &temporary_id).await?;
    if row_text(&row, "send_status").as_deref() != Some("sent") {
        return Err(format!(
            "message {temporary_id} row did not settle sent: {:?}",
            row_text(&row, "send_status")
        ));
    }
    let bus_envelope = to_bus_envelope(&received);
    if bus_event_name(&received).as_deref() != Some("im:post:received") {
        return Err("NativeEventSink event cannot map to im:post:received bus channel".to_string());
    }
    if bus_envelope.get("channel").and_then(Value::as_str) != Some("im:post:received") {
        return Err(format!("bad native bus envelope: {bus_envelope}"));
    }

    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(10), engine)
        .await
        .map_err(|_| "Helix engine graceful shutdown timed out".to_string())?
        .map_err(|error| format!("Helix engine task failed: {error}"))?;
    actor_transport
        .close()
        .await
        .map_err(|error| format!("close actor WS: {error}"))?;
    observer
        .transport
        .close()
        .await
        .map_err(|error| format!("close observer WS: {error}"))?;
    remove_test_storage(&db_path);
    Ok(SendEvidence {
        temporary_id,
        sending,
        received,
        row,
        bus_envelope,
    })
}

/// 在 engine 上发送一个不存在的 canonical channel，验证 send-failed 而非假 received。
async fn run_helix_send_failure(
    context: &AuthContext,
    invalid_channel_id: &str,
) -> Result<(), String> {
    let (storage, db_path) = open_test_storage().await?;
    let storage_for_engine = storage.clone();
    let (event_sink, mut events) = NativeEventSink::new();
    let http = go_http(&context.actor).await?;
    let mut config = ImConfig::default();
    config.ws_url = GO_WS_URL.to_string();
    config.api_base_url = GO_HTTP_BASE.to_string();
    config.default_api_base_url = main_api_base();
    config.auth_user_id = context.actor.user_id.clone();
    config.company_id = context.actor.company_id.clone();
    config.send_timeout_ms = 2_000;
    config.ping_interval_ms = 60_000;
    let mut module = ImModule::new(config);
    module.register_channel(
        helix_im::state::ChannelId::from_str(invalid_channel_id)
            .ok_or_else(|| "failure test channel id is not canonical".to_string())?,
        0,
    );
    let mut shell = ExecutionShell::new();
    shell.register(module);
    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(256);
    let mut transport =
        NativeTransport::new(GO_WS_URL, TransportId::from_raw(31), Some(tick_tx.clone()))
            .with_handshake_headers(ws_headers(&context.actor));
    let activation = transport
        .connect()
        .await
        .map_err(|error| format!("failure actor WS connect failed: {error}"))?;
    let transport = Arc::new(transport);
    let mut transports = TransportTable::new();
    transports.insert(TransportId::from_raw(31), Arc::clone(&transport));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let engine = tokio::spawn(run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx.clone(),
        EngineConfig::new(
            storage_for_engine,
            http,
            SharedFileUploader::default(),
            event_sink,
            8,
        ),
        shutdown_rx,
        transports,
    ));
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
    let failed_row = scan_message_row(&storage, &failed_temporary_id).await?;
    let failed_status = row_text(&failed_row, "send_status");
    if failed_status.as_deref() != Some("unsend") && failed_status.as_deref() != Some("failed") {
        return Err(format!(
            "failed message {failed_temporary_id} row has unexpected status: {failed_status:?}"
        ));
    }
    let _ = shutdown_tx.send(());
    timeout(Duration::from_secs(10), engine)
        .await
        .map_err(|_| "failure engine graceful shutdown timed out".to_string())?
        .map_err(|error| format!("failure engine task failed: {error}"))?;
    transport
        .close()
        .await
        .map_err(|error| format!("close failure actor WS: {error}"))?;
    remove_test_storage(&db_path);
    Ok(())
}

/// 断言直接 Go post 回声保留指定字段，不把 HTTP response 当 WS 证据。
fn assert_post_frame(frame: &Value, message_type: &str, text: &str) -> Result<(), String> {
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

/// 为每个真实 gate 提供清晰的未实现原因；忽略项不会伪装成通过。
macro_rules! explicit_real_gap {
    ($name:ident, $reason:literal) => {
        #[tokio::test]
        #[ignore = $reason]
        async fn $name() {
            panic!("NOT_VERIFIED: {}", $reason);
        }
    };
}

mod phase1 {
    use super::*;

    mod channel {
        use super::*;

        /// MV3-G08a：真实 Go channel/create SUCCESS 必须返回可供 WS/发送使用的 canonical id。
        #[tokio::test]
        async fn mv3_g08a_create_channel_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context)
                .await
                .unwrap_or_else(|error| panic!("MV3-G08a BLOCKED/FAIL: {error}"));
            assert!(
                channel_id.len() >= 20,
                "channel/create returned non-canonical id"
            );
            close_channel(&context, &channel_id)
                .await
                .unwrap_or_else(|error| panic!("MV3-G08a cleanup failed: {error}"));
        }
    }

    mod post_send {
        use super::*;

        /// MV3-G01a + G02a：完整 HTTP admission→WS post→Helix terminal→SQLite→emit 链。
        #[tokio::test]
        async fn mv3_g01a_g02a_text_send_real_chain() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context)
                .await
                .unwrap_or_else(|error| panic!("MV3-G01a setup failed: {error}"));
            let result = run_helix_send(
                &context,
                &channel_id,
                SendIntent {
                    message_type: "TEXT",
                    text: resource_name("text"),
                    props: json!({}),
                    viewers: vec!["all".to_string()],
                    mentions: Vec::new(),
                    replied: None,
                },
                15_000,
            )
            .await;
            close_channel(&context, &channel_id)
                .await
                .unwrap_or_else(|error| panic!("MV3-G01a cleanup failed: {error}"));
            let evidence = result.unwrap_or_else(|error| panic!("MV3-G01a/G02a FAIL: {error}"));
            let received: Value = serde_json::from_slice(evidence.received.0.as_ref()).unwrap();
            assert_eq!(received["event"], "im:post:received");
            assert_eq!(
                row_text(&evidence.row, "send_status").as_deref(),
                Some("sent")
            );
            assert_eq!(evidence.bus_envelope["channel"], "im:post:received");
            assert!(!evidence.temporary_id.is_empty());
            assert!(!evidence.sending.0.is_empty());
        }

        /// MV3-G01b：TEMPLATE 由 Helix command 规范化后打到真实 Go，并经真实 post 回声确认。
        #[tokio::test]
        async fn mv3_g01b_template_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context).await.unwrap();
            let text = resource_name("template");
            let result = run_helix_send(
                &context,
                &channel_id,
                SendIntent {
                    message_type: "TEMPLATE",
                    text: text.clone(),
                    props: json!({}),
                    viewers: vec!["all".to_string()],
                    mentions: vec!["all".to_string()],
                    replied: None,
                },
                15_000,
            )
            .await;
            close_channel(&context, &channel_id).await.unwrap();
            let evidence = result.unwrap_or_else(|error| panic!("MV3-G01b FAIL: {error}"));
            assert_eq!(row_text(&evidence.row, "type").as_deref(), Some("TEMPLATE"));
            let received: Value = serde_json::from_slice(evidence.received.0.as_ref()).unwrap();
            assert_eq!(data_string(&received, &["message"]), Some(text));
        }

        /// MV3-G01f：DOCUMENT props 走 Helix 出站规范化后打到真实 Go。
        #[tokio::test]
        async fn mv3_g01f_document_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context).await.unwrap();
            let text = resource_name("document");
            let result = run_helix_send(
                &context,
                &channel_id,
                SendIntent {
                    message_type: "DOCUMENT",
                    text: text.clone(),
                    props: json!({
                        "document": {
                            "snapshot": {"type": "doc", "content": []}
                        }
                    }),
                    viewers: vec!["all".to_string()],
                    mentions: Vec::new(),
                    replied: None,
                },
                15_000,
            )
            .await;
            close_channel(&context, &channel_id).await.unwrap();
            let evidence = result.unwrap_or_else(|error| panic!("MV3-G01f FAIL: {error}"));
            let props = row_text(&evidence.row, "props").unwrap_or_default();
            assert!(
                props.contains("document"),
                "durable row lost DOCUMENT props: {props}"
            );
            assert_eq!(row_text(&evidence.row, "type").as_deref(), Some("DOCUMENT"));
        }

        /// MV3-G01k：先造真实 parent，再以 Helix replied 四锚发送并核对 durable reply columns。
        #[tokio::test]
        async fn mv3_g01k_reply_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context).await.unwrap();
            let parent = run_helix_send(
                &context,
                &channel_id,
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
            .await
            .unwrap_or_else(|error| panic!("MV3-G01k parent FAIL: {error}"));
            let parent_value: Value = serde_json::from_slice(parent.received.0.as_ref()).unwrap();
            let parent_id = data_string(&parent_value, &["id"])
                .expect("parent received event must have server id");
            let reply_text = resource_name("reply");
            let result = run_helix_send(
                &context,
                &channel_id,
                SendIntent {
                    message_type: "TEXT",
                    text: reply_text,
                    props: json!({}),
                    viewers: vec!["all".to_string()],
                    mentions: Vec::new(),
                    replied: Some(json!({
                        "id": parent_id,
                        "reply_id": "",
                        "reply_root_id": "",
                        "reply_first_level_id": ""
                    })),
                },
                15_000,
            )
            .await;
            close_channel(&context, &channel_id).await.unwrap();
            let evidence = result.unwrap_or_else(|error| panic!("MV3-G01k FAIL: {error}"));
            assert!(!row_text(&evidence.row, "reply_id")
                .unwrap_or_default()
                .is_empty());
        }

        /// MV3-G01l：定向 viewers 通过真实三用户 WS 观察者验证可见性，不只断 HTTP 200。
        #[tokio::test]
        async fn mv3_g01l_targeted_visibility_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel_id = create_channel(&context).await.unwrap();
            let mut observer = open_raw_ws(&context.observer, 40).await.unwrap();
            let mut third = open_raw_ws(&context.third, 41).await.unwrap();
            wait_ws_hello(&mut observer).await.unwrap();
            wait_ws_hello(&mut third).await.unwrap();
            let text = resource_name("targeted");
            let http = go_http(&context.actor).await.unwrap();
            let temporary_id = run_token("targeted");
            let observation = request_json(
                &http,
                "POST",
                &format!("{GO_HTTP_BASE}/posts/create"),
                Some(json!({
                    "channelId": channel_id,
                    "userId": context.actor.user_id,
                    "type": "TEXT",
                    "message": text,
                    "simpleMessage": text,
                    "props": {},
                    "viewers": [context.observer.user_id],
                    "mentions": [],
                    "temporaryId": temporary_id
                })),
                "targeted",
            )
            .await
            .unwrap();
            require_success(observation, "posts/create targeted").unwrap();
            let observer_frame = wait_ws_action(
                &mut observer,
                "post",
                |frame| {
                    extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            assert_post_frame(&observer_frame, "TEXT", &text).unwrap();
            let third_result = wait_ws_action(
                &mut third,
                "post",
                |frame| {
                    extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                        == Some(temporary_id.as_str())
                },
                Duration::from_secs(3),
            )
            .await;
            observer.transport.close().await.unwrap();
            third.transport.close().await.unwrap();
            close_channel(&context, &channel_id).await.unwrap();
            assert!(
                third_result.is_err(),
                "third user unexpectedly received targeted post: {third_result:?}"
            );
        }

        /// MV3-G02b：真实无效 channel admission 不得释放 received，必须送出 send-failed。
        #[tokio::test]
        async fn mv3_g02b_failed_send_real_go() {
            let (_guard, context) = remote_setup().await;
            let invalid_channel = format!(
                "chfixx{:020x}",
                RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            );
            run_helix_send_failure(&context, &invalid_channel)
                .await
                .unwrap_or_else(|error| panic!("MV3-G02b FAIL: {error}"));
        }

        // MV3-G02c：retry 的实时重发仍需单独绑定真实 failure row；当前目标保留为显式 gap。
        explicit_real_gap!(
            mv3_g02c_retry_send_real_go,
            "retry requires a prior durable failed row and approved real retry fixture"
        );
    }

    mod post_actions {
        use super::*;

        /// MV3-G03a/G03b：用真实 source post + createPosts 转发到另一个 run-owned channel。
        #[tokio::test]
        async fn mv3_g03a_g03b_forward_real_go() {
            let (_guard, context) = remote_setup().await;
            let source = create_channel(&context).await.unwrap();
            let target = create_channel(&context).await.unwrap();
            let (ack, source_frame) = direct_post(
                &context,
                &source,
                "TEXT",
                &resource_name("forward-source"),
                json!({}),
                json!(["all"]),
                json!([]),
            )
            .await
            .unwrap();
            let source_id = extract_id(&source_frame, &["id"])
                .or_else(|| extract_id(&ack, &["id"]))
                .expect("source post must have server id");
            let mut target_observer = open_raw_ws(&context.observer, 50).await.unwrap();
            wait_ws_hello(&mut target_observer).await.unwrap();
            let http = go_http(&context.actor).await.unwrap();
            let observation = request_json(
                &http,
                "POST",
                &format!("{GO_HTTP_BASE}/posts/createPosts"),
                Some(json!({ "postIds": [source_id], "channelIds": [target] })),
                "forward",
            )
            .await
            .unwrap();
            require_success(observation, "posts/createPosts").unwrap();
            let target_frame = wait_ws_action(
                &mut target_observer,
                "post",
                |frame| extract_id(frame, &["id"]).is_some(),
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            assert_eq!(
                string_field(&target_frame, &["action"]).as_deref(),
                Some("post")
            );
            target_observer.transport.close().await.unwrap();
            close_channel(&context, &source).await.unwrap();
            close_channel(&context, &target).await.unwrap();
        }

        /// MV3-G05a：真实撤回 endpoint 必须在 WS 发布 post_update/revoke 事实。
        #[tokio::test]
        async fn mv3_g05a_revoke_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let (ack, frame) = direct_post(
                &context,
                &channel,
                "TEXT",
                &resource_name("revoke"),
                json!({}),
                json!(["all"]),
                json!([]),
            )
            .await
            .unwrap();
            let post_id = extract_id(&frame, &["id"])
                .or_else(|| extract_id(&ack, &["id"]))
                .unwrap();
            let mut observer = open_raw_ws(&context.observer, 51).await.unwrap();
            wait_ws_hello(&mut observer).await.unwrap();
            let http = go_http(&context.actor).await.unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/revoke"),
                    Some(json!({ "postId": post_id })),
                    "revoke",
                )
                .await
                .unwrap(),
                "posts/revoke",
            )
            .unwrap();
            let update = wait_ws_action(
                &mut observer,
                "posts_update",
                |frame| extract_id(frame, &["id"]).is_some(),
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            assert!(update.to_string().contains("revoke"));
            observer.transport.close().await.unwrap();
            close_channel(&context, &channel).await.unwrap();
        }

        /// MV3-G05b：真实 quickReply endpoint 的 WS 更新必须带 quickReply 事实。
        #[tokio::test]
        async fn mv3_g05b_quick_reply_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let (ack, frame) = direct_post(
                &context,
                &channel,
                "TEXT",
                &resource_name("quick-reply"),
                json!({}),
                json!(["all"]),
                json!([]),
            )
            .await
            .unwrap();
            let post_id = extract_id(&frame, &["id"])
                .or_else(|| extract_id(&ack, &["id"]))
                .unwrap();
            let mut observer = open_raw_ws(&context.observer, 52).await.unwrap();
            wait_ws_hello(&mut observer).await.unwrap();
            let http = go_http(&context.actor).await.unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/quickReply"),
                    Some(json!({ "postId": post_id, "emoji": "👍" })),
                    "quick-reply",
                )
                .await
                .unwrap(),
                "posts/quickReply",
            )
            .unwrap();
            let update = wait_ws_action(
                &mut observer,
                "post_update",
                |frame| frame.to_string().contains("quickReply"),
                Duration::from_secs(20),
            )
            .await
            .unwrap();
            assert!(update.to_string().contains("👍"));
            observer.transport.close().await.unwrap();
            close_channel(&context, &channel).await.unwrap();
        }

        /// MV3-G05c/G05d：真实 urgentPost→urgentConfirm 两步都必须返回 SUCCESS。
        #[tokio::test]
        async fn mv3_g05c_g05d_urgent_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let (ack, frame) = direct_post(
                &context,
                &channel,
                "TEXT",
                &resource_name("urgent"),
                json!({}),
                json!(["all"]),
                json!([]),
            )
            .await
            .unwrap();
            let post_id = extract_id(&frame, &["id"])
                .or_else(|| extract_id(&ack, &["id"]))
                .unwrap();
            let http = go_http(&context.actor).await.unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/urgentPost"),
                    Some(json!({
                        "channelId": channel,
                        "postId": post_id,
                        "targetIds": [context.observer.user_id]
                    })),
                    "urgent-post",
                )
                .await
                .unwrap(),
                "posts/urgentPost",
            )
            .unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/urgentConfirm"),
                    Some(json!({ "postId": post_id })),
                    "urgent-confirm",
                )
                .await
                .unwrap(),
                "posts/urgentConfirm",
            )
            .unwrap();
            close_channel(&context, &channel).await.unwrap();
        }

        /// MV3-G05e：templateReceived/read 两个真实 endpoint 都必须收到业务 SUCCESS。
        #[tokio::test]
        async fn mv3_g05e_template_received_and_read_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let (ack, frame) = direct_post(
                &context,
                &channel,
                "TEMPLATE",
                &resource_name("template-received"),
                json!({ "template": { "type": "TEXT", "text": "template" } }),
                json!(["all"]),
                json!(["all"]),
            )
            .await
            .unwrap();
            let post_id = extract_id(&frame, &["id"])
                .or_else(|| extract_id(&ack, &["id"]))
                .unwrap();
            let http = go_http(&context.actor).await.unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/post/templateReceived"),
                    Some(json!({ "postId": post_id })),
                    "template-received",
                )
                .await
                .unwrap(),
                "post/templateReceived",
            )
            .unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/post/read"),
                    Some(json!({ "channelId": channel, "posts": [post_id] })),
                    "post-read",
                )
                .await
                .unwrap(),
                "post/read",
            )
            .unwrap();
            close_channel(&context, &channel).await.unwrap();
        }

        // MV3-G01h/G01i/G01j 当前 registry 明确为 gap，不能以未知接口返回 200 伪装通过。
        explicit_real_gap!(
            mv3_g01h_rich_post_real_go,
            "rich post gate requires approved Java/media gateway and canonical object-store fixture"
        );
        explicit_real_gap!(
            mv3_g01i_voice_post_real_go,
            "voice post gate is deferred in the Phase1 registry"
        );
        explicit_real_gap!(
            mv3_g01j_video_post_real_go,
            "video post gate is deferred in the Phase1 registry"
        );
        explicit_real_gap!(
            mv3_g05l_zero_write_real_go,
            "zero-write is NOT_APPLICABLE and has no Go mutation to verify"
        );
    }

    mod upload {
        use super::*;

        /// 历史 Go-owned media prepare→PUT→complete→get 探针，不是当前 Angular 媒体真链。
        ///
        /// Angular 现在由 Java/fileDrive/OSS 完成字节上传，Go 只消费稳定媒体引用；该旧探针
        /// 保留用于后端媒体 API 独立诊断并继续忽略，不能作为 Phase1 G01c/d/e/g/G03c 证据。
        #[tokio::test]
        #[ignore = "legacy Go-owned media API probe; Angular media owner is Java/fileDrive/OSS"]
        async fn mv3_media_upload_chain_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let content = format!("{}\n", resource_name("media")).into_bytes();
            let sha256 = format!("{:x}", sha2::Sha256::digest(&content));
            let http = go_http(&context.actor).await.unwrap();
            let prepared = require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/media/upload/prepare"),
                    Some(json!({
                        "fileName": "phase1.txt",
                        "contentType": "text/plain",
                        "size": content.len(),
                        "sha256": sha256,
                        "channelId": channel
                    })),
                    "media-prepare",
                )
                .await
                .unwrap(),
                "media/upload/prepare",
            )
            .unwrap();
            let data = prepared.get("data").unwrap_or(&prepared);
            let upload_url = string_field(data, &["url", "uploadUrl"])
                .expect("media prepare must return upload url");
            let upload_id = string_field(data, &["uploadId", "upload_id"])
                .expect("media prepare must return uploadId");
            let upload_http = anonymous_http(GO_HTTP_BASE).await.unwrap();
            let put = upload_http
                .request(HttpRequest {
                    method: "PUT".to_string(),
                    url: upload_url,
                    headers: vec![("Content-Type".to_string(), "text/plain".to_string())],
                    body: Some(Bytes::from(content)),
                })
                .await
                .unwrap();
            assert!(
                put.status < 300,
                "media PUT failed with HTTP {}",
                put.status
            );
            let completed = require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/media/upload/complete"),
                    Some(json!({ "uploadId": upload_id })),
                    "media-complete",
                )
                .await
                .unwrap(),
                "media/upload/complete",
            )
            .unwrap();
            let file_id = extract_id(&completed, &["fileId", "file_id"])
                .expect("media complete must return fileId");
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/media/get"),
                    Some(json!({ "fileId": file_id })),
                    "media-get",
                )
                .await
                .unwrap(),
                "media/get",
            )
            .unwrap();
            close_channel(&context, &channel).await.unwrap();
        }
    }

    mod schedule {
        use super::*;

        /// MV3-G04a/G04b/G04c/G04d/G04e：真实 schedule create/get/cancel 与普通发送资源隔离。
        #[tokio::test]
        async fn mv3_g04a_to_g04e_schedule_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let http = go_http(&context.actor).await.unwrap();
            let schedule_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + 120_000;
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/createSchedule"),
                    Some(json!({
                        "post": {
                            "channelId": channel,
                            "userId": context.actor.user_id,
                            "type": "TEXT",
                            "message": resource_name("scheduled"),
                            "simpleMessage": "scheduled"
                        },
                        "schedulePostAt": schedule_at
                    })),
                    "schedule-create",
                )
                .await
                .unwrap(),
                "posts/createSchedule",
            )
            .unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/getSchedule"),
                    Some(json!({ "channelId": channel, "userId": context.actor.user_id })),
                    "schedule-get",
                )
                .await
                .unwrap(),
                "posts/getSchedule",
            )
            .unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/cancelSchedule"),
                    Some(json!({ "channelId": channel })),
                    "schedule-cancel",
                )
                .await
                .unwrap(),
                "posts/cancelSchedule",
            )
            .unwrap();
            close_channel(&context, &channel).await.unwrap();
        }
    }

    mod draft {
        // MV3-G02e 是 Host-local 草稿，不存在 Go 8066 endpoint；保留真实目标的显式 non-go gap。
        explicit_real_gap!(
            mv3_g02e_draft_real_go,
            "draft is local Host storage by contract; no Go 8066 endpoint exists"
        );
    }

    mod announcement {
        use super::*;

        /// MV3-G07b：真实 announcement/save admission 必须返回 SUCCESS。
        #[tokio::test]
        async fn mv3_g07b_announcement_save_real_go() {
            let (_guard, context) = remote_setup().await;
            let channel = create_channel(&context).await.unwrap();
            let http = go_http(&context.actor).await.unwrap();
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/post/announcement/save"),
                    Some(json!({
                        "channelId": channel,
                        "userId": context.actor.user_id,
                        "type": "ANNOUNCEMENT",
                        "message": resource_name("announcement"),
                        "props": {"announcement": true}
                    })),
                    "announcement-save",
                )
                .await
                .unwrap(),
                "post/announcement/save",
            )
            .unwrap();
            close_channel(&context, &channel).await.unwrap();
        }
    }
}
