//! http_cross — 出站 HTTP 横切层（鉴权注入 / 401 / 断网 / traceparent 透传）。
//!
//! ## 归属（P1.5，ADR-007 双端共享）
//!
//! 现网（cses-client）这套横切在浏览器 Angular `networkInterceptor` + `app-http.service`：
//! 鉴权头注入 / 401→logout / 断网提示 / traceparent 出站透传。接管后这些全下沉 driver，
//! 且因 PC native 与 Flutter FFI **共享出站语义**，落在 host 层（非 native-only），native/ffi
//! 复用同一份——否则两端鉴权 / 错误分类语义漂移。
//!
//! ## 设计：装饰器（decorator over `HttpRequester`）
//!
//! `CrossCuttingHttp<H, E>` 包内层 `H: HttpRequester`（真出站，如 `SharedHttpClient`）+
//! `E: EventSink`（失败码回报通道）+ `AuthTokenRegistry`（driver 持有的真 token）。它**自身**
//! 实现 `HttpRequester`，故能透明插进事件泵的 `http` 端口位置，对 core 不可见。
//!
//! 每条出站请求：
//! 1. **鉴权注入**：读 core 标的意图头 `X-Auth-Kind: session|bot`（[`helix_core::AUTH_KIND_HEADER`]），
//!    据此从 registry 取真 token 注入对应鉴权头，并 **strip 意图头**（绝不出网，HX-C001/C6）。
//!    core/helix-im 永不持 token——`grep 'mp_|Authorization|Bearer'` 在 core/im 必空。
//! 2. **traceparent 透传**：core 已把 `traceparent` 放进 `req.headers`（链路头），横切层
//!    **原样保留**到出站请求——不改不删（出站头 == 入站信封 trace，端到端可断言）。
//! 3. **失败码回报**：内层返回 4xx==401 → emit `im:http:unauthorized`；内层 `PortError::Transport`
//!    （断网 / 超时 / 连接拒绝）→ emit `im:net:offline`，随后把错误原样上抛（不吞）。
//!    其余 status / 错误透传，不旁路 emit。

use std::sync::Arc;

use bytes::Bytes;
use parking_lot::RwLock; // 无 poison：panic 不静默丢 token（对齐 network.rs HostHeaderRegistry）
use serde_json::json;

use helix_core::auth::AuthKind;
use helix_core::effect::{DomainEventBytes, HttpRequest, HttpResponse};
use helix_core::ports::{EventSink, HttpRequester};
use helix_core::{PortError, AUTH_KIND_HEADER};

/// 失败码回报的总线事件名（经 `to_bus_envelope` 取 `event` 作 channel → 前端 dispatcher）。
pub const EVENT_HTTP_UNAUTHORIZED: &str = "im:http:unauthorized";
/// 断网回报的总线事件名。
pub const EVENT_NET_OFFLINE: &str = "im:net:offline";

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

/// 出站 HTTP 横切装饰器：鉴权注入 + 401/断网 emit + traceparent 透传。
#[derive(Clone)]
pub struct CrossCuttingHttp<H, E> {
    inner: H,
    event_sink: Arc<E>,
    auth: AuthTokenRegistry,
}

impl<H, E> CrossCuttingHttp<H, E>
where
    H: HttpRequester,
    E: EventSink,
{
    pub fn new(inner: H, event_sink: Arc<E>, auth: AuthTokenRegistry) -> Self {
        Self {
            inner,
            event_sink,
            auth,
        }
    }

    /// 鉴权头注入意图的纯函数（无 I/O）：读 `X-Auth-Kind` → 注真 token → strip 意图头。
    /// traceparent 等其余头**原样保留**（透传）。返回改写后的请求 + 解析出的意图（供测试断言）。
    ///
    /// 意图头缺失 / 未知值 → 不注入（零信任不臆测），仅 strip 任何残留意图头。
    fn apply_auth(&self, mut req: HttpRequest) -> (HttpRequest, Option<AuthKind>) {
        // 取意图（大小写不敏感匹配头名，对齐 HTTP 头语义）。
        let kind = req
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_KIND_HEADER))
            .and_then(|(_, value)| AuthKind::from_str(value));

        // strip 全部意图头（绝不出网）——保留其余头（含 traceparent，透传）。
        req.headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case(AUTH_KIND_HEADER));

        // 命中意图且 registry 有 token → 注入真鉴权头（覆盖同名旧值，去重后置）。
        if let Some(kind) = kind {
            if let Some((auth_name, auth_value)) = self.auth.resolve(kind) {
                req.headers
                    .retain(|(name, _)| !name.eq_ignore_ascii_case(&auth_name));
                req.headers.push((auth_name, auth_value));
            }
        }

        (req, kind)
    }

    fn emit_event(&self, event: &str, detail: serde_json::Value) {
        let payload = json!({ "event": event, "data": detail });
        // 序列化失败不可能（json! 构造），兜底空对象不 panic（边界零信任）。
        let bytes = serde_json::to_vec(&payload).unwrap_or_default();
        self.event_sink.emit(DomainEventBytes(Bytes::from(bytes)));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl<H, E> HttpRequester for CrossCuttingHttp<H, E>
where
    H: HttpRequester,
    E: EventSink,
{
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let (prepared, _kind) = self.apply_auth(req);
        let url = prepared.url.clone();

        match self.inner.request(prepared).await {
            Ok(resp) => {
                // 401 → 鉴权失效，回报让前端 logout（业务下沉，前端不再自判）。
                if resp.status == 401 {
                    self.emit_event(
                        EVENT_HTTP_UNAUTHORIZED,
                        json!({ "status": resp.status, "url": url }),
                    );
                }
                Ok(resp)
            }
            Err(e) => {
                // 断网 / 超时 / 连接拒绝 = Transport：回报离线提示，错误**原样上抛**（不吞，
                // 上层 inflight 闸 / 重连对账仍据此走）。Http / 其余错误不旁路 emit。
                if matches!(e, PortError::Transport(_)) {
                    self.emit_event(EVENT_NET_OFFLINE, json!({ "url": url }));
                }
                Err(e)
            }
        }
    }
}
