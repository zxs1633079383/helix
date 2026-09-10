//! http_cross — 出站 HTTP 横切层（鉴权注入 / outcome observer / traceparent 透传）。
//!
//! ## 归属（P1.5，ADR-007 双端共享）
//!
//! 现网（cses-client）这套横切在浏览器 Angular `networkInterceptor` + `app-http.service`：
//! 鉴权头注入 / 401→logout / 断网提示 / traceparent 出站透传。接管后鉴权与 OTel 执行下沉
//! host，HTTP outcome 的业务解释由模块注入；PC native 与 Flutter FFI 复用同一装饰器，避免
//! 两端执行语义漂移。
//!
//! ## 设计：装饰器（decorator over `HttpRequester`）
//!
//! `CrossCuttingHttp<H, E>` 包内层 `H: HttpRequester`（真出站，如 `SharedHttpClient`）+
//! `E: EventSink`（业务事件回报通道）+ `AuthTokenRegistry`（driver 持有的真 token）+
//! `HttpOutcomeObserver`（由业务模块在装配点注入）。它**自身**实现 `HttpRequester`，故能透明
//! 插进事件泵的 `http` 端口位置，对 core 不可见。
//!
//! 每条出站请求：
//! 1. **鉴权注入**：读 core 标的意图头 `X-Auth-Kind: session|bot`（[`helix_core::AUTH_KIND_HEADER`]），
//!    据此从 registry 取真 token 注入对应鉴权头，并 **strip 意图头**（绝不出网，HX-C001/C6）。
//!    core/helix-im 永不持 token——`grep 'mp_|Authorization|Bearer'` 在 core/im 必空。
//! 2. **traceparent 透传**：core 已把 `traceparent` 放进 `req.headers`（链路头），横切层
//!    **原样保留**到出站请求——不改不删（出站头 == 入站信封 trace，端到端可断言）。
//! 3. **结果观察**：把完整 HTTP outcome 交给业务 observer；若返回业务事件则 emit。host 不认识
//!    事件名或 payload，错误仍原样上抛（不吞）。

use std::sync::Arc;

use parking_lot::RwLock; // 无 poison：panic 不静默丢 token（对齐 network.rs HostHeaderRegistry）

use crate::base64_encode;
use crate::otel::{HostOtelRuntime, HostSpanScope, TraceDirection};
use crate::trace::TraceCarrier;
use helix_core::auth::{apply_auth_intent, AuthKind};
use helix_core::effect::{DomainEventBytes, HttpRequest, HttpResponse};
use helix_core::ports::{EventSink, HttpRequester};
use helix_core::PortError;

/// 平台 HTTP outcome 到业务事件的观察函数。
///
/// 规则与 payload 归业务模块；host 只调用它并把返回事件送入 `EventSink`。
pub type HttpOutcomeObserver =
    fn(url: &str, outcome: &Result<HttpResponse, PortError>) -> Option<DomainEventBytes>;

/// driver 持有的真鉴权 token（平台壳登录后写入；core 永不触碰）。
///
/// 两类凭据各一槽，按 [`AuthKind`] 路由。`Arc<RwLock<_>>` 运行时可热换（token 轮换 / 重登），
/// 与出站请求并发安全；写少读多，`parking_lot::RwLock` 足够。
#[derive(Clone, Default)]
pub struct AuthTokenRegistry {
    inner: Arc<RwLock<AuthSlots>>,
}

#[derive(Default)]
struct AuthSlots {
    /// session 凭据出站头：`(header_name, header_value)`。`None` = 未登录 / 未注入。
    session: Option<(String, String)>,
    /// bot 凭据出站头（如 `("Authorization", "Bearer mp_xxx")`）。
    bot: Option<(String, String)>,
}

impl AuthTokenRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注入 / 覆盖 session 凭据出站头（平台壳登录后调；如 `("cookieId", "<userId>")`）。
    pub fn set_session(&self, header_name: impl Into<String>, header_value: impl Into<String>) {
        self.inner.write().session = Some((header_name.into(), header_value.into()));
    }

    /// 注入 / 覆盖 bot 凭据出站头（如 `("Authorization", "Bearer mp_...")`）。
    pub fn set_bot(&self, header_name: impl Into<String>, header_value: impl Into<String>) {
        self.inner.write().bot = Some((header_name.into(), header_value.into()));
    }

    /// 清空某类凭据（如 401 logout 后）。
    pub fn clear(&self, kind: AuthKind) {
        let mut slots = self.inner.write();
        match kind {
            AuthKind::Session => slots.session = None,
            AuthKind::Bot => slots.bot = None,
        }
    }

    /// 取某类凭据当前出站头快照（横切层注入用）。
    fn resolve(&self, kind: AuthKind) -> Option<(String, String)> {
        let slots = self.inner.read();
        match kind {
            AuthKind::Session => slots.session.clone(),
            AuthKind::Bot => slots.bot.clone(),
        }
    }
}

/// 出站 HTTP 横切装饰器：鉴权注入 + 业务 outcome 观察 + traceparent 透传。
#[derive(Clone)]
pub struct CrossCuttingHttp<H, E> {
    inner: H,
    event_sink: Arc<E>,
    auth: AuthTokenRegistry,
    outcome_observer: HttpOutcomeObserver,
    otel: Option<HostOtelRuntime>,
}

impl<H, E> CrossCuttingHttp<H, E>
where
    H: HttpRequester,
    E: EventSink,
{
    pub fn new(
        inner: H,
        event_sink: Arc<E>,
        auth: AuthTokenRegistry,
        outcome_observer: HttpOutcomeObserver,
    ) -> Self {
        Self {
            inner,
            event_sink,
            auth,
            outcome_observer,
            otel: None,
        }
    }

    pub fn with_otel(mut self, otel: HostOtelRuntime) -> Self {
        self.otel = Some(otel).filter(HostOtelRuntime::is_enabled);
        self
    }

    pub fn is_otel_enabled(&self) -> bool {
        self.otel.is_some()
    }

    /// 鉴权头注入意图的纯函数（无 I/O）：读 `X-Auth-Kind` → 注真 token → strip 意图头。
    /// traceparent 等其余头**原样保留**（透传）。返回改写后的请求 + 解析出的意图（供测试断言）。
    ///
    /// 意图头缺失 / 未知值 → 不注入（零信任不臆测），仅 strip 任何残留意图头。
    fn apply_auth(&self, req: HttpRequest) -> (HttpRequest, Option<AuthKind>) {
        apply_auth_intent(req, |kind| self.auth.resolve(kind))
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<H, E> HttpRequester for CrossCuttingHttp<H, E>
where
    H: HttpRequester,
    E: EventSink,
{
    /// 执行鉴权后的 HTTP 请求，并把 response/error 作为 request span 的显式子节点。
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let (prepared, _kind) = self.apply_auth(req);
        let url = prepared.url.clone();
        let carrier = TraceCarrier::from_headers(&prepared.headers);
        let request_scope = self.otel.as_ref().map(|runtime| {
            runtime.span_with_attributes(
                "helix.http.request",
                TraceDirection::Outbound,
                carrier.as_ref(),
                full_debug_http_attributes(runtime, &prepared),
            )
        });
        let outcome = self.inner.request(prepared).await;
        // 优先使用 Go 响应携带的服务端 Span；没有响应头时仍回退到本地 request child。
        let response_parent = match &outcome {
            Ok(response) => TraceCarrier::from_response_headers(&response.headers),
            Err(_) => None,
        }
        .or_else(|| {
            request_scope
                .as_ref()
                .and_then(HostSpanScope::child_carrier)
        })
        .or_else(|| carrier.clone());
        let _response_scope = match &outcome {
            Ok(response) => self.otel.as_ref().map(|runtime| {
                let mut attributes =
                    vec![("http.response.status_code", response.status.to_string())];
                if runtime.is_full_debug() {
                    attributes.push((
                        "helix.debug.http.response_headers",
                        bounded_debug_string(
                            &serde_json::to_string(&response.headers)
                                .unwrap_or_else(|_| "[]".to_string()),
                        ),
                    ));
                    attributes.push((
                        "helix.debug.http.response_body_base64",
                        bounded_debug_bytes(&response.body),
                    ));
                }
                runtime.span_with_attributes(
                    "helix.http.response",
                    TraceDirection::Inbound,
                    response_parent.as_ref(),
                    attributes,
                )
            }),
            Err(error) => self.otel.as_ref().map(|runtime| {
                runtime.span_with_attributes(
                    "helix.http.response",
                    TraceDirection::Inbound,
                    response_parent.as_ref(),
                    vec![("helix.http.error_kind", error_kind(error).to_string())],
                )
            }),
        };
        if let Some(event) = (self.outcome_observer)(&url, &outcome) {
            self.event_sink.emit(event);
        }
        outcome
    }
}

/// 将 HTTP 失败折叠成低基数错误类别，不把响应正文或凭据写入 Trace。
fn error_kind(error: &PortError) -> &'static str {
    match error {
        PortError::Storage(_) => "storage",
        PortError::StorageConflict => "storage_conflict",
        PortError::Transport(_) => "transport",
        PortError::Http(_) => "http",
        PortError::Clock(_) => "clock",
        PortError::IdSource(_) => "id_source",
        PortError::Other(_) => "other",
    }
}

const FULL_DEBUG_PAYLOAD_LIMIT: usize = 64 * 1024;

/// safe 模式返回空属性；full_debug 才将鉴权后请求送入受控 OTLP span。
fn full_debug_http_attributes(
    runtime: &HostOtelRuntime,
    request: &HttpRequest,
) -> Vec<(&'static str, String)> {
    if !runtime.is_full_debug() {
        return Vec::new();
    }
    let mut attributes = vec![
        (
            "helix.debug.http.request_headers",
            bounded_debug_string(
                &serde_json::to_string(&request.headers).unwrap_or_else(|_| "[]".to_string()),
            ),
        ),
        (
            "helix.debug.http.request_url",
            bounded_debug_string(&request.url),
        ),
        (
            "helix.debug.http.request_method",
            bounded_debug_string(&request.method),
        ),
    ];
    if let Some(body) = &request.body {
        attributes.push((
            "helix.debug.http.request_body_base64",
            bounded_debug_bytes(body),
        ));
    }
    attributes
}

/// 限制 full_debug 文本长度，避免一个异常请求撑爆 exporter queue。
fn bounded_debug_string(value: &str) -> String {
    if value.len() <= FULL_DEBUG_PAYLOAD_LIMIT {
        return value.to_string();
    }
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= FULL_DEBUG_PAYLOAD_LIMIT)
        .last()
        .unwrap_or(0);
    format!(
        "{}...[truncated {} bytes]",
        &value[..boundary],
        value.len() - boundary
    )
}

/// 将二进制响应限制并编码为可携带的受控 Debug 属性。
fn bounded_debug_bytes(bytes: &[u8]) -> String {
    let shown = bytes.len().min(FULL_DEBUG_PAYLOAD_LIMIT);
    let mut value = base64_encode(&bytes[..shown]);
    if shown < bytes.len() {
        value.push_str(&format!("...[truncated {} bytes]", bytes.len() - shown));
    }
    value
}
