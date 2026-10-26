//! HTTP and render-ready effects for older-context paging.

use bytes::Bytes;
use helix_core::effect::HttpRequest;
use helix_core::{Correlation, Effect};
use serde_json::Value;

/// 组装一轮 `posts/postContext` POST Effect（IM 网关 base + connectionId 身份头，与既有 outbound 同壳）。
/// 真凭据由 driver 注入（C6，core 不拼）；body 由 [`build_post_context_body`] 产。
pub fn post_context_http(
    base_url: &str,
    body: &Value,
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    post_context_http_tracked(base_url, body, corr, connection_id, None)
}

/// 与 [`post_context_http`] 相同，并将可选 host correlation 仅写入 `Cses-Track-Id`。
pub fn post_context_http_tracked(
    base_url: &str,
    body: &Value,
    corr: Correlation,
    connection_id: Option<&str>,
    request_id: Option<&str>,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::sync_http_effects::session_auth_headers(
        connection_id,
    ));
    if let Some(request_id) = request_id.filter(|value| !value.is_empty()) {
        headers.push(("Cses-Track-Id".to_string(), request_id.to_string()));
    }
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{base_url}/posts/postContext"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(body).unwrap_or_default())),
        },
    }
}
