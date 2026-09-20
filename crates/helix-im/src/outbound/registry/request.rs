use bytes::Bytes;
use helix_core::effect::{Effect, HttpRequest};
use helix_core::{AuthKind, Correlation, AUTH_KIND_HEADER};
use serde_json::Value;

const CSES_TRACK_ID_HEADER: &str = "Cses-Track-Id";

/// 组装 HTTP 请求，并只下标鉴权意图；真凭据仍由 driver 注入。
pub(super) fn http_request(
    base: &str,
    method: &str,
    path: &str,
    body: Value,
    auth_kind: AuthKind,
    connection_id: Option<&str>,
    request_id: Option<&str>,
    corr: Correlation,
) -> Effect {
    // CATEGORY profile input is business data; its message projections are cleaned separately.
    let body = if path.starts_with("post-chain/") && path.ends_with("?mode=CATEGORY") {
        body
    } else {
        crate::message_identity::sanitize(body)
    };
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::to_effect::connection_id_headers(connection_id));
    headers.push((AUTH_KIND_HEADER.to_string(), auth_kind.as_str().to_string()));
    if let Some(request_id) = request_id.filter(|value| !value.is_empty()) {
        headers.push((CSES_TRACK_ID_HEADER.to_string(), request_id.to_string()));
    }
    Effect::Http {
        corr,
        req: HttpRequest {
            method: method.to_string(),
            url: format!("{base}/{path}"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())),
        },
    }
}
