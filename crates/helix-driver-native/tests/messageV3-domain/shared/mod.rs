#![allow(dead_code)]
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
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, OnceCell};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

pub(crate) const GO_HTTP_BASE: &str = "http://127.0.0.1:8066/api/cses";
pub(crate) const GO_WS_URL: &str = "ws://127.0.0.1:8066/api/v4/websocket";
pub(crate) const MAIN_API_BASE: &str = "http://cses-server-paas25-test.cses.svc.cluster.local:3399";
pub(crate) const GO_HEALTH_URL: &str = "http://127.0.0.1:8066/api/cses/health";
pub(crate) const CREDENTIALS_RELATIVE: &str =
    "angular/visual/cses-client/.wdio/credentials.local.json";

pub(crate) static RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);
pub(crate) static AUTH_CONTEXT: OnceCell<Result<Arc<AuthContext>, String>> = OnceCell::const_new();
pub(crate) static REMOTE_LOCK: OnceCell<Mutex<()>> = OnceCell::const_new();

#[derive(Debug, Deserialize)]
pub(crate) struct CredentialFile {
    pub(crate) accounts: Vec<CredentialAccount>,
    #[serde(default, alias = "defaultIndex")]
    pub(crate) default_index: usize,
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

pub(crate) struct AuthContext {
    pub(crate) actor: Persona,
    pub(crate) observer: Persona,
    pub(crate) third: Persona,
}

#[derive(Debug)]
pub(crate) struct HttpObservation {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct RawWs {
    pub(crate) transport: Arc<NativeTransport>,
    pub(crate) ticks: mpsc::Receiver<Tick>,
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

pub(crate) struct SendEvidence {
    pub(crate) temporary_id: String,
    pub(crate) sending: DomainEventBytes,
    pub(crate) received: DomainEventBytes,
    pub(crate) row: Row,
    pub(crate) bus_envelope: Value,
}

/// 持有一次真实 Helix native engine 会话；所有 Phase2-4 command Gate 共用这一条边界。
///
/// 会话同时拥有 actor WS、NativeHttp、NativeStorage、NativeEventSink 和 engine task，
/// 因而测试不会把“直接 curl 成功”误报成 Helix 业务完成。调用方必须在断言前调用
/// [`RealHelixSession::command`]，并在返回前调用 [`RealHelixSession::close`]。
pub(crate) struct RealHelixSession {
    pub(crate) storage: NativeStorage,
    pub(crate) events: broadcast::Receiver<DomainEventBytes>,
    tick_tx: mpsc::Sender<Tick>,
    actor_transport: Arc<NativeTransport>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    engine: Option<JoinHandle<()>>,
    db_path: PathBuf,
}

impl RealHelixSession {
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

    /// 发送 shutdown、等待 engine drain、关闭 actor WS，并删除本轮独占 SQLite。
    pub(crate) async fn close(mut self) -> Result<(), String> {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        if let Some(engine) = self.engine.take() {
            timeout(Duration::from_secs(15), engine)
                .await
                .map_err(|_| "real Helix engine graceful shutdown timed out".to_string())?
                .map_err(|error| format!("real Helix engine task failed: {error}"))?;
        }
        self.actor_transport
            .close()
            .await
            .map_err(|error| format!("close real Helix actor WS: {error}"))?;
        remove_test_storage(&self.db_path);
        Ok(())
    }
}

/// 启动一条注册指定频道的真实 Helix native engine 会话，并等待 authenticated hello。
pub(crate) async fn start_real_helix_session(
    context: &AuthContext,
    channel_ids: &[&str],
) -> Result<RealHelixSession, String> {
    let (storage, db_path) = open_test_storage().await?;
    let (event_sink, events) = NativeEventSink::new();
    let http = go_http(&context.actor).await?;

    let mut im_config = ImConfig::default();
    im_config.ws_url = GO_WS_URL.to_string();
    im_config.api_base_url = GO_HTTP_BASE.to_string();
    im_config.default_api_base_url = MAIN_API_BASE.to_string();
    im_config.auth_user_id = context.actor.user_id.clone();
    im_config.company_id = context.actor.company_id.clone();
    im_config.user_name = context.actor.user_name.clone();
    im_config.org_name = context.actor.org_name.clone();
    im_config.dept_name = context.actor.dept_name.clone();
    im_config.ping_interval_ms = 60_000;
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
    let transport_raw =
        100u32.saturating_add((RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 10_000) as u32);
    let mut actor_transport = NativeTransport::new(
        GO_WS_URL,
        TransportId::from_raw(transport_raw),
        Some(tick_tx.clone()),
    )
    .with_handshake_headers(ws_headers(&context.actor));
    let activation = actor_transport
        .connect()
        .await
        .map_err(|error| format!("real Helix actor WS connect failed: {error}"))?;
    let actor_transport = Arc::new(actor_transport);
    let mut transports = TransportTable::new();
    transports.insert(
        TransportId::from_raw(transport_raw),
        Arc::clone(&actor_transport),
    );

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let engine_tick_tx = tick_tx.clone();
    let engine_storage = storage.clone();
    let engine = tokio::spawn(async move {
        run_engine_loop_with_transports(
            shell,
            tick_rx,
            engine_tick_tx,
            EngineConfig::new(
                engine_storage,
                http,
                SharedFileUploader::default(),
                event_sink,
                8,
            )
            .with_metric_sink(Arc::new(NoopMetricSink)),
            shutdown_rx,
            transports,
        )
        .await;
    });
    activation
        .activate()
        .await
        .map_err(|error| format!("real Helix actor WS activation failed: {error}"))?;

    let mut session = RealHelixSession {
        storage,
        events,
        tick_tx,
        actor_transport,
        shutdown_tx: Some(shutdown_tx),
        engine: Some(engine),
        db_path,
    };
    session
        .wait_event(
            "im:connection:established",
            |_| true,
            Duration::from_secs(20),
        )
        .await?;
    Ok(session)
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

/// 在固定候选路径中寻找 Angular 现有凭据，不从环境变量或用户输入读取 secret。
pub(crate) fn discover_credentials_path() -> Result<PathBuf, String> {
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
pub(crate) async fn anonymous_http(base: &str) -> Result<NativeHttp, String> {
    NativeHttp::with_config(HostNetworkConfig::new(base, GO_WS_URL))
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
    let http = anonymous_http(MAIN_API_BASE).await?;
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
        &format!("{MAIN_API_BASE}/User/login"),
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
pub(crate) async fn discover_auth_context() -> Result<AuthContext, String> {
    let credentials_path = discover_credentials_path()?;
    let content = std::fs::read_to_string(&credentials_path)
        .map_err(|error| format!("read credentials file failed: {error}"))?;
    let credentials: CredentialFile = serde_json::from_str(&content)
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
pub(crate) async fn go_http(persona: &Persona) -> Result<NativeHttp, String> {
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
            let action = frame.get("action").and_then(Value::as_str);
            if actions.iter().any(|candidate| Some(*candidate) == action) && predicate(&frame) {
                return Ok(frame);
            }
        }
    })
    .await
    .map_err(|_| format!("timed out waiting real Go WS actions {actions:?}"))?
}

/// 只等待一次 hello，作为 WS authenticated session 的业务前置证据。
pub(crate) async fn wait_ws_hello(ws: &mut RawWs) -> Result<Value, String> {
    wait_ws_action(ws, "hello", |_| true, Duration::from_secs(10)).await
}

/// 等待指定领域事件，并按 data/temporaryId 做精确匹配。
pub(crate) async fn wait_domain_event<F>(
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
pub(crate) fn data_string(envelope: &Value, keys: &[&str]) -> Option<String> {
    let data = envelope.get("data")?;
    string_field(data, keys)
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

/// 以 canonical channel/create body 创建 run-owned 私有群聊。
pub(crate) async fn create_channel(context: &AuthContext) -> Result<String, String> {
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
pub(crate) async fn close_channel(context: &AuthContext, channel_id: &str) -> Result<(), String> {
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
    let mut observer = open_raw_ws(watcher, raw_id).await?;
    wait_ws_hello(&mut observer).await?;
    let temporary_id = run_token("post");
    let http = go_http(sender).await?;
    let body = json!({
        "channelId": channel_id,
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
    let storage = NativeStorage::open(&url)
        .await
        .map_err(|error| format!("open test SQLite failed: {error}"))?;
    apply_im_schema(&storage).await?;
    Ok((storage, path))
}

/// 删除本测试自己的 SQLite 文件，不触碰共享客户端数据库。
pub(crate) fn remove_test_storage(path: &Path) {
    let _ = std::fs::remove_file(path);
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

/// 读取 row 的文本列，避免把 Debug 输出当成 storage 证据。
pub(crate) fn row_text(row: &Row, column: &str) -> Option<String> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Text(text)) if name == column => Some(text.clone()),
            _ => None,
        })
}

/// 按真实 engine 装配一次完整 send，返回 HTTP→WS→SQLite→emit 的逐跳证据。
pub(crate) async fn run_helix_send(
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
    config.default_api_base_url = MAIN_API_BASE.to_string();
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
    let row =
        wait_message_status(&storage, &temporary_id, &["sent"], Duration::from_secs(8)).await?;
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
pub(crate) async fn run_helix_send_failure(
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
    config.default_api_base_url = MAIN_API_BASE.to_string();
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
    let _failed_row = wait_message_status(
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

/// 为单个 Gate 写入不含 secret 的机器可读证据，便于审计 PASS/失败分类。
pub(crate) fn write_gate_evidence(
    phase: &str,
    domain: &str,
    gate: &str,
    status: &str,
    details: Value,
) -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/messagev3-phase1-4-real-chain");
    let run_id = format!("{}-{}", run_token("evidence"), std::process::id());
    let path = root.join(&run_id).join(phase).join(domain).join(gate);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("create evidence directory failed: {error}"))?;
    let file = path.join("evidence.json");
    let payload = json!({
        "phase": phase,
        "domain": domain,
        "gate": gate,
        "status": status,
        "details": details,
    });
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|error| format!("serialize evidence failed: {error}"))?;
    std::fs::write(&file, bytes).map_err(|error| format!("write evidence failed: {error}"))?;
    Ok(file)
}

/// 运行标准文本/模板/文档发送链并断言 Persist、Go WS、terminal emit 与 SQLite。
pub(crate) async fn assert_send_chain(
    context: &AuthContext,
    phase: &str,
    domain: &str,
    gate: &str,
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
    write_gate_evidence(
        phase,
        domain,
        gate,
        "PASS",
        json!({
            "command": "im_send_message",
            "effect": ["Persist", "NativeHttp", "NativeTransport"],
            "persist_before": "send_status=sending",
            "http_business": "status=SUCCESS",
            "ws_action": "post",
            "projection": "im:post:received",
            "persist_after": "send_status=sent",
            "event_sink": "im:__bus__",
            "temporary_id": evidence.temporary_id,
        }),
    )?;
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
    let result = assert_send_chain(
        &context,
        phase,
        domain,
        gate,
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
    let cleanup = close_channel(&context, &channel_id).await;
    if let Err(error) = cleanup {
        return Err(format!("channel cleanup failed: {error}"));
    }
    result
}

/// 记录一个明确未验证 Gate，测试通过 #[ignore] 保留真实前置缺口而不伪造 PASS。
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
    let target = create_channel(&context).await?;
    let result = async {
        let mut source_posts = Vec::new();
        for index in 0..2 {
            let (ack, frame) = direct_post(
                &context,
                &source,
                "TEXT",
                &format!("{}-{}", resource_name("forward-source"), index),
                json!({}),
                json!(["all"]),
                json!([]),
            )
            .await?;
            let post = frame
                .get("data")
                .cloned()
                .or_else(|| ack.get("data").cloned())
                .ok_or_else(|| "source post frame has no data object".to_string())?;
            source_posts.push(post);
        }
        let mut observer = open_raw_ws(&context.observer, 70).await?;
        wait_ws_hello(&mut observer).await?;
        let http = go_http(&context.actor).await?;
        let mode = if mode_selector == 1 { "item" } else { "merge" };
        let posts = if mode == "item" {
            source_posts.clone()
        } else {
            let items = source_posts
                .iter()
                .map(|post| {
                    json!({
                        "id": extract_id(post, &["id"]).unwrap_or_default(),
                        "temporaryId": extract_id(post, &["temporaryId", "temporary_id"]).unwrap_or_default(),
                        "userId": context.actor.user_id,
                        "type": "TEXT",
                        "message": string_field(post, &["message"]).unwrap_or_default(),
                        "simpleMessage": string_field(post, &["simpleMessage", "simple_message"]).unwrap_or_default(),
                        "props": post.get("props").cloned().unwrap_or_else(|| json!({})),
                        "viewers": ["all"],
                        "createAt": post.get("createAt").cloned().unwrap_or(Value::from(0)),
                    })
                })
                .collect::<Vec<_>>();
            let merge_message = serde_json::to_string(&items)
                .map_err(|error| format!("serialize mergeMessage: {error}"))?;
            vec![json!({
                "id": "",
                "temporaryId": run_token("merge-forward"),
                "channelId": "",
                "userId": context.actor.user_id,
                "teamId": context.actor.company_id,
                "type": "MULTIPLY",
                "message": "",
                "simpleMessage": "[聊天记录]",
                "props": {
                    "forwardTitle": "真实来源频道的聊天记录",
                    "sourceChannelId": source,
                    "sourceChannelName": "真实来源频道",
                    "mergeMessage": merge_message,
                },
                "mentions": [],
                "viewers": ["all"],
                "topicId": "",
                "revoke": false,
                "createAt": 0
            })]
        };
        let response = request_json(
            &http,
            "POST",
            &format!("{GO_HTTP_BASE}/posts/createPosts"),
            Some(json!({ "posts": posts, "channelIds": [target] })),
            "forward",
        )
        .await?;
        require_success(response, "posts/createPosts")?;
        if mode == "item" {
            for source_post in &source_posts {
                let expected = string_field(source_post, &["message"]).unwrap_or_default();
                let frame = wait_ws_action(
                    &mut observer,
                    "post",
                    |frame| string_field(frame, &["message"]).as_deref() == Some(expected.as_str())
                        || data_string(frame, &["message"]).as_deref() == Some(expected.as_str()),
                    Duration::from_secs(20),
                )
                .await?;
                if !frame.to_string().contains(&expected) {
                    return Err("item forward post did not retain source message".to_string());
                }
            }
        } else {
            let frame = wait_ws_action(
                &mut observer,
                "post",
                |frame| {
                    string_field(frame, &["type"]).as_deref() == Some("MULTIPLY")
                        || data_string(frame, &["type"]).as_deref() == Some("MULTIPLY")
                },
                Duration::from_secs(20),
            )
            .await?;
            if !frame.to_string().contains("mergeMessage") {
                return Err("merge forward post did not retain mergeMessage".to_string());
            }
        }
        observer
            .transport
            .close()
            .await
            .map_err(|error| format!("close forward observer WS: {error}"))?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "http": "posts/createPosts",
                "wire": "posts + channelIds",
                "mode": mode,
                "source_post_count": source_posts.len(),
                "ws_action": "post",
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    for channel in [&target, &source] {
        let _ = close_channel(&context, channel).await;
    }
    result
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
    let result = async {
        let (ack, frame) = direct_post(
            &context,
            &channel,
            "TEXT",
            &resource_name(gate),
            json!({}),
            json!(["all"]),
            json!([]),
        )
        .await?;
        let post_id = extract_id(&frame, &["id"])
            .or_else(|| extract_id(&ack, &["id"]))
            .ok_or_else(|| "post action source has no post id".to_string())?;
        let mut observer = open_raw_ws(&context.observer, 71).await?;
        wait_ws_hello(&mut observer).await?;
        let http = go_http(&context.actor).await?;
        let mut request_body = body;
        if let Some(map) = request_body.as_object_mut() {
            map.entry("postId".to_string())
                .or_insert_with(|| Value::String(post_id));
            if map
                .get("channelId")
                .and_then(Value::as_str)
                .map(str::is_empty)
                .unwrap_or(true)
            {
                map.insert("channelId".to_string(), Value::String(channel.clone()));
            }
            if endpoint == "posts/urgentPost" && !map.contains_key("targetIds") {
                map.insert("targetIds".to_string(), json!([context.observer.user_id]));
            }
            if endpoint == "posts/urgentConfirm" && !map.contains_key("channelId") {
                map.insert("channelId".to_string(), Value::String(channel.clone()));
            }
        }
        if endpoint == "posts/urgentConfirm" {
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/posts/urgentPost"),
                    Some(json!({
                        "channelId": channel,
                        "postId": request_body
                            .get("postId")
                            .cloned()
                            .unwrap_or(Value::Null),
                        "targetIds": [context.observer.user_id],
                    })),
                    "urgent-seed",
                )
                .await?,
                "posts/urgentPost",
            )?;
            let _seed = wait_ws_actions(
                &mut observer,
                &["update_channel", "post_update"],
                |_| true,
                Duration::from_secs(20),
            )
            .await?;
        }
        require_success(
            request_json(
                &http,
                "POST",
                &format!("{GO_HTTP_BASE}/{endpoint}"),
                Some(request_body),
                gate,
            )
            .await?,
            endpoint,
        )?;
        let update = if endpoint.starts_with("posts/urgent") {
            wait_ws_actions(
                &mut observer,
                &["update_channel", "post_update"],
                |frame| {
                    needle
                        .map(|value| frame.to_string().contains(value))
                        .unwrap_or(true)
                },
                Duration::from_secs(20),
            )
            .await?
        } else {
            wait_ws_action(
                &mut observer,
                action,
                |frame| {
                    needle
                        .map(|value| frame.to_string().contains(value))
                        .unwrap_or(true)
                },
                Duration::from_secs(20),
            )
            .await?
        };
        if let Some(value) = needle {
            if !update.to_string().contains(value) {
                return Err(format!("WS update did not contain expected marker {value}"));
            }
        }
        observer
            .transport
            .close()
            .await
            .map_err(|error| format!("close action observer WS: {error}"))?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({ "http": endpoint, "ws_action": action, "marker": needle }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
}

/// 调用真实 schedule 三步 endpoint，供每个 schedule Gate 独立复用同一真实前置。
pub(crate) async fn assert_schedule_chain(
    phase: &str,
    domain: &str,
    gate: &str,
    step: &str,
) -> Result<(), String> {
    let (_guard, context) = remote_setup().await;
    let channel = create_channel(&context).await?;
    let result = async {
        let http = go_http(&context.actor).await?;
        let schedule_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_millis() as i64
            + 120_000;
        let (endpoint, body) = match step {
            "create" => (
                "posts/createSchedule",
                json!({
                    "post": {
                        "channelId": channel,
                        "userId": context.actor.user_id,
                        "type": "TEXT",
                        "message": resource_name("scheduled"),
                        "simpleMessage": "scheduled"
                    },
                    "schedulePostAt": schedule_at
                }),
            ),
            "get" => (
                "posts/getSchedule",
                json!({ "channelId": channel, "userId": context.actor.user_id }),
            ),
            "cancel" => ("posts/cancelSchedule", json!({ "channelId": channel })),
            _ => return Err(format!("unknown schedule step {step}")),
        };
        require_success(
            request_json(
                &http,
                "POST",
                &format!("{GO_HTTP_BASE}/{endpoint}"),
                Some(body),
                gate,
            )
            .await?,
            endpoint,
        )?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({ "http": endpoint, "business_status": "SUCCESS" }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
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
        let http = go_http(&context.actor).await?;
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
                    "props": { "announcement": true }
                })),
                gate,
            )
            .await?,
            "post/announcement/save",
        )?;
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({ "http": "post/announcement/save", "business_status": "SUCCESS" }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
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
                    "id": parent_id,
                    "reply_id": "",
                    "reply_root_id": "",
                    "reply_first_level_id": ""
                })),
            },
            15_000,
        )
        .await?;
        if row_text(&reply.row, "reply_id")
            .unwrap_or_default()
            .is_empty()
        {
            return Err("reply durable row has empty reply_id".to_string());
        }
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "command": "im_send_message(replied)",
                "parent_id_present": true,
                "persist_after": "reply_id non-empty",
                "projection": "im:post:received",
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
    let result = async {
        let mut observer = open_raw_ws(&context.observer, 72).await?;
        let mut third = open_raw_ws(&context.third, 73).await?;
        wait_ws_hello(&mut observer).await?;
        wait_ws_hello(&mut third).await?;
        let text = resource_name("targeted");
        let temporary_id = run_token("targeted");
        let http = go_http(&context.actor).await?;
        require_success(
            request_json(
                &http,
                "POST",
                &format!("{GO_HTTP_BASE}/posts/create"),
                Some(json!({
                    "channelId": channel,
                    "userId": context.actor.user_id,
                    "type": "TEXT",
                    "message": text,
                    "simpleMessage": text,
                    "props": {},
                    "viewers": [context.observer.user_id],
                    "mentions": [],
                    "temporaryId": temporary_id
                })),
                gate,
            )
            .await?,
            "posts/create targeted",
        )?;
        let frame = wait_ws_action(
            &mut observer,
            "post",
            |frame| {
                extract_id(frame, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await?;
        assert_post_frame(&frame, "TEXT", &text)?;
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
        observer
            .transport
            .close()
            .await
            .map_err(|error| format!("close targeted observer WS: {error}"))?;
        third
            .transport
            .close()
            .await
            .map_err(|error| format!("close targeted third WS: {error}"))?;
        if third_result.is_ok() {
            return Err("non-target third user received targeted post".to_string());
        }
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "http": "posts/create",
                "ws_target": "observer",
                "ws_negative": "third user did not receive post",
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
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
        let (ack, frame) = direct_post(
            &context,
            &channel,
            "TEMPLATE",
            &resource_name("template-received"),
            json!({ "template": { "type": "TEXT", "text": "template" } }),
            json!(["all"]),
            json!(["all"]),
        )
        .await?;
        let post_id = extract_id(&frame, &["id"])
            .or_else(|| extract_id(&ack, &["id"]))
            .ok_or_else(|| "template post has no id".to_string())?;
        let http = go_http(&context.actor).await?;
        for (endpoint, body) in [
            ("post/templateReceived", json!({ "postId": post_id })),
            (
                "post/read",
                json!({ "channelId": channel, "posts": [post_id] }),
            ),
        ] {
            require_success(
                request_json(
                    &http,
                    "POST",
                    &format!("{GO_HTTP_BASE}/{endpoint}"),
                    Some(body),
                    gate,
                )
                .await?,
                endpoint,
            )?;
        }
        write_gate_evidence(
            phase,
            domain,
            gate,
            "PASS",
            json!({
                "http": ["post/templateReceived", "post/read"],
                "post_id_present": true,
                "business_status": "SUCCESS",
            }),
        )?;
        Ok::<(), String>(())
    }
    .await;
    let _ = close_channel(&context, &channel).await;
    result
}
