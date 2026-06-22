//! P1.5 出站横切层验收（回归锚: P1.5）—— 鉴权注入 / 401 / 断网 / traceparent 透传。
//!
//! 测对象 = `helix_driver_host::CrossCuttingHttp`（decorator over HttpRequester）。用
//! 录制型 mock 内层（记录收到的出站请求 + 可编程响应/错误）+ 录制型 EventSink（捕获 emit），
//! 端到端断言四件横切行为。覆盖维度对齐 03-测试策略 P1.5 行 + 02 路线 P1.5 gate。

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;

use helix_core::auth::{AuthKind, AUTH_KIND_HEADER};
use helix_core::effect::{DomainEventBytes, HttpRequest, HttpResponse};
use helix_core::ports::{EventSink, HttpRequester};
use helix_core::PortError;
use helix_driver_host::{
    AuthTokenRegistry, CrossCuttingHttp, EVENT_HTTP_UNAUTHORIZED, EVENT_NET_OFFLINE,
};

// ─── mock 内层 HttpRequester：记录收到的出站请求 + 可编程结果 ─────────────────────

#[derive(Clone)]
struct MockInner {
    seen: Arc<Mutex<Vec<HttpRequest>>>,
    result: Arc<Mutex<Result<HttpResponse, PortError>>>,
}

impl MockInner {
    fn new(result: Result<HttpResponse, PortError>) -> Self {
        Self {
            seen: Arc::new(Mutex::new(Vec::new())),
            result: Arc::new(Mutex::new(result)),
        }
    }

    fn ok(status: u16) -> Self {
        Self::new(Ok(HttpResponse {
            status,
            headers: vec![],
            body: Bytes::new(),
        }))
    }

    /// 取内层实际收到的（横切改写后的）出站请求快照。
    fn last_seen(&self) -> HttpRequest {
        self.seen.lock().unwrap().last().cloned().expect("a request")
    }
}

#[async_trait]
impl HttpRequester for MockInner {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        self.seen.lock().unwrap().push(req);
        match &*self.result.lock().unwrap() {
            Ok(resp) => Ok(resp.clone()),
            Err(e) => Err(clone_err(e)),
        }
    }
}

fn clone_err(e: &PortError) -> PortError {
    match e {
        PortError::Transport(m) => PortError::Transport(m.clone()),
        PortError::Http(m) => PortError::Http(m.clone()),
        PortError::Storage(m) => PortError::Storage(m.clone()),
        other => PortError::Other(other.to_string()),
    }
}

// ─── 录制型 EventSink：捕获 emit 的 DomainEventBytes ──────────────────────────────

#[derive(Clone, Default)]
struct RecordingSink {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl RecordingSink {
    /// 解析所有 emit 出的事件为 JSON（`{event, data}`）。
    fn emitted(&self) -> Vec<serde_json::Value> {
        self.events.lock().unwrap().clone()
    }

    /// 提取所有 emit 事件的 `event` 名（== to_bus_envelope 的 channel）。
    fn event_names(&self) -> Vec<String> {
        self.emitted()
            .iter()
            .filter_map(|v| v.get("event").and_then(|e| e.as_str()).map(String::from))
            .collect()
    }
}

impl EventSink for RecordingSink {
    fn emit(&self, event: DomainEventBytes) {
        let v: serde_json::Value = serde_json::from_slice(event.0.as_ref())
            .expect("emit 的字节必须是合法 JSON（横切层产 {event,data}）");
        self.events.lock().unwrap().push(v);
    }
}

// ─── 装配工具 ────────────────────────────────────────────────────────────────────

fn build(
    inner: MockInner,
    auth: AuthTokenRegistry,
) -> (CrossCuttingHttp<MockInner, RecordingSink>, RecordingSink) {
    let sink = RecordingSink::default();
    let cross = CrossCuttingHttp::new(inner, Arc::new(sink.clone()), auth);
    (cross, sink)
}

fn req_with_headers(headers: Vec<(&str, &str)>) -> HttpRequest {
    HttpRequest {
        method: "POST".to_string(),
        url: "/api/cses/posts/create".to_string(),
        headers: headers
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: None,
    }
}

fn header_value<'a>(req: &'a HttpRequest, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

// ─── 1. 鉴权注入：session 意图 → 注真 token + strip 意图头 ─────────────────────────

#[tokio::test]
async fn session_intent_injects_token_and_strips_intent_header() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "676cc4ccfbbc501161d5cd65");
    let (cross, sink) = build(inner.clone(), auth);

    let req = req_with_headers(vec![(AUTH_KIND_HEADER, AuthKind::Session.as_str())]);
    cross.request(req).await.expect("ok");

    let seen = inner.last_seen();
    // 真 token 注入。
    assert_eq!(
        header_value(&seen, "cookieId"),
        Some("676cc4ccfbbc501161d5cd65")
    );
    // 意图头被 strip（绝不出网，C6）。
    assert!(
        header_value(&seen, AUTH_KIND_HEADER).is_none(),
        "X-Auth-Kind 意图头必须 strip，绝不出网"
    );
    // 200 不旁路 emit。
    assert!(sink.event_names().is_empty());
}

#[tokio::test]
async fn bot_intent_injects_bearer_token() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_bot("Authorization", "Bearer mp_secret123");
    let (cross, _sink) = build(inner.clone(), auth);

    let req = req_with_headers(vec![(AUTH_KIND_HEADER, AuthKind::Bot.as_str())]);
    cross.request(req).await.expect("ok");

    let seen = inner.last_seen();
    assert_eq!(header_value(&seen, "Authorization"), Some("Bearer mp_secret123"));
    assert!(header_value(&seen, AUTH_KIND_HEADER).is_none());
}

// ─── 2. traceparent 出站透传：req 头里的 traceparent 原样到内层（== 入站 trace）───────

#[tokio::test]
async fn traceparent_passes_through_unchanged() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "u1");
    let (cross, _sink) = build(inner.clone(), auth);

    // 模拟入站信封带的 trace（真 Go fixture post.json 同形：00-<trace>-<span>-01）。
    let inbound_trace = "00-000000000000000040c638ca85a38ded-6eb425e6be47edf8-01";
    let req = req_with_headers(vec![
        (AUTH_KIND_HEADER, AuthKind::Session.as_str()),
        ("traceparent", inbound_trace),
    ]);
    cross.request(req).await.expect("ok");

    let seen = inner.last_seen();
    // 端到端断言：出站头 == 入站信封 trace（横切层不改不删 traceparent）。
    assert_eq!(
        header_value(&seen, "traceparent"),
        Some(inbound_trace),
        "traceparent 必须原样透传到出站头"
    );
}

// ─── 3. 401 → emit im:http:unauthorized ──────────────────────────────────────────

#[tokio::test]
async fn status_401_emits_unauthorized() {
    let inner = MockInner::ok(401);
    let auth = AuthTokenRegistry::new();
    let (cross, sink) = build(inner, auth);

    let resp = cross.request(req_with_headers(vec![])).await.expect("ok resp");
    assert_eq!(resp.status, 401);

    // emit 了 im:http:unauthorized（event 名 == to_bus_envelope channel）。
    assert_eq!(sink.event_names(), vec![EVENT_HTTP_UNAUTHORIZED.to_string()]);
    let ev = &sink.emitted()[0];
    assert_eq!(ev["event"], EVENT_HTTP_UNAUTHORIZED);
    assert_eq!(ev["data"]["status"], 401);
}

#[tokio::test]
async fn status_403_does_not_emit_unauthorized() {
    // 仅 401 触发回报；403/其余 4xx 不旁路 emit（业务层判）。
    let inner = MockInner::ok(403);
    let (cross, sink) = build(inner, AuthTokenRegistry::new());
    cross.request(req_with_headers(vec![])).await.expect("ok");
    assert!(sink.event_names().is_empty());
}

// ─── 4. 断网（Transport error）→ emit im:net:offline + 错误原样上抛 ───────────────

#[tokio::test]
async fn transport_error_emits_offline_and_propagates() {
    let inner = MockInner::new(Err(PortError::Transport(
        "network: connection refused".to_string(),
    )));
    let (cross, sink) = build(inner, AuthTokenRegistry::new());

    let result = cross.request(req_with_headers(vec![])).await;
    // 错误不吞，原样上抛（上层 inflight 闸 / 重连对账据此走）。
    assert!(matches!(result, Err(PortError::Transport(_))));
    // emit 了 im:net:offline。
    assert_eq!(sink.event_names(), vec![EVENT_NET_OFFLINE.to_string()]);
}

#[tokio::test]
async fn http_error_does_not_emit_offline() {
    // 非 Transport 的错误（如 Http）不触发离线回报。
    let inner = MockInner::new(Err(PortError::Http("invalid method".to_string())));
    let (cross, sink) = build(inner, AuthTokenRegistry::new());
    let result = cross.request(req_with_headers(vec![])).await;
    assert!(matches!(result, Err(PortError::Http(_))));
    assert!(sink.event_names().is_empty());
}

// ─── 5. 边界：意图缺失 / 未知值 / token 缺失 → 不注入（零信任不臆测）─────────────

#[tokio::test]
async fn missing_intent_header_injects_nothing() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "u1"); // 有 token，但请求无意图头 → 不注入。
    let (cross, _sink) = build(inner.clone(), auth);

    cross.request(req_with_headers(vec![])).await.expect("ok");
    let seen = inner.last_seen();
    assert!(header_value(&seen, "cookieId").is_none());
}

#[tokio::test]
async fn unknown_intent_value_strips_header_and_injects_nothing() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "u1");
    let (cross, _sink) = build(inner.clone(), auth);

    // 未知意图值（如 "admin"）→ from_str None → 不注入，但意图头仍 strip。
    let req = req_with_headers(vec![(AUTH_KIND_HEADER, "admin")]);
    cross.request(req).await.expect("ok");
    let seen = inner.last_seen();
    assert!(header_value(&seen, AUTH_KIND_HEADER).is_none());
    assert!(header_value(&seen, "cookieId").is_none());
}

#[tokio::test]
async fn intent_present_but_no_token_registered_injects_nothing() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new(); // 空 registry。
    let (cross, _sink) = build(inner.clone(), auth);

    let req = req_with_headers(vec![(AUTH_KIND_HEADER, AuthKind::Bot.as_str())]);
    cross.request(req).await.expect("ok");
    let seen = inner.last_seen();
    // 意图头 strip；无 token 可注入 → 无 Authorization。
    assert!(header_value(&seen, AUTH_KIND_HEADER).is_none());
    assert!(header_value(&seen, "Authorization").is_none());
}

// ─── 6. token 覆盖：同名旧鉴权头被真 token 顶替（去重后置）─────────────────────────

#[tokio::test]
async fn injected_token_overrides_stale_same_name_header() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "fresh");
    let (cross, _sink) = build(inner.clone(), auth);

    // 请求里已带一个旧 cookieId → 注入应顶替为 fresh（不重复留两条）。
    let req = req_with_headers(vec![
        (AUTH_KIND_HEADER, AuthKind::Session.as_str()),
        ("cookieId", "stale"),
    ]);
    cross.request(req).await.expect("ok");
    let seen = inner.last_seen();
    let cookies: Vec<&str> = seen
        .headers
        .iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("cookieId"))
        .map(|(_, v)| v.as_str())
        .collect();
    assert_eq!(cookies, vec!["fresh"], "同名旧鉴权头须被真 token 顶替，不重复");
}

// ─── 7. registry clear：401 logout 后清凭据 → 后续不再注入 ────────────────────────

#[tokio::test]
async fn registry_clear_removes_credentials() {
    let inner = MockInner::ok(200);
    let auth = AuthTokenRegistry::new();
    auth.set_bot("Authorization", "Bearer mp_x");
    auth.clear(AuthKind::Bot);
    let (cross, _sink) = build(inner.clone(), auth);

    let req = req_with_headers(vec![(AUTH_KIND_HEADER, AuthKind::Bot.as_str())]);
    cross.request(req).await.expect("ok");
    assert!(header_value(&inner.last_seen(), "Authorization").is_none());
}

#[tokio::test]
async fn registry_clear_session_independent_of_bot() {
    let auth = AuthTokenRegistry::new();
    auth.set_session("cookieId", "u1");
    auth.set_bot("Authorization", "Bearer mp_x");
    auth.clear(AuthKind::Session); // 只清 session，bot 保留。

    let inner = MockInner::ok(200);
    let (cross, _sink) = build(inner.clone(), auth);
    let req = req_with_headers(vec![(AUTH_KIND_HEADER, AuthKind::Bot.as_str())]);
    cross.request(req).await.expect("ok");
    assert_eq!(
        header_value(&inner.last_seen(), "Authorization"),
        Some("Bearer mp_x")
    );
}
