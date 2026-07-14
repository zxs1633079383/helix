//! 本地 wire 调试日志，独立于 OTel。
//!
//! `HELIX_DUMP_NETWORK=1` 同时开启 HTTP/WS；也可分别用 `HELIX_DUMP_HTTP=1`、
//! `HELIX_DUMP_WS=1`（兼容 `HELIX_DUMP_FRAMES=1`）。凭据头默认脱敏；仅本地明确设置
//! `HELIX_DUMP_NETWORK_SENSITIVE=1` 时显示 cookie/Authorization/token 原值；敏感开关
//! 不会单独启用 dump，必须同时开启一个 scope 开关。

use std::collections::BTreeMap;

use bytes::Bytes;

use crate::network::PreparedHttpRequest;

const PREVIEW_LIMIT: usize = 64 * 1024;
const REDACTED: &str = "<redacted>";

pub(super) fn dump_http_request(request: &PreparedHttpRequest) {
    let Some(mode) = DumpMode::http() else {
        return;
    };
    let headers = visible_headers(&request.headers, mode.sensitive);
    let data = request
        .body
        .as_ref()
        .map(|data| preview_payload(data, mode.sensitive))
        .unwrap_or_else(|| "<none>".to_string());
    let url = visible_url(&request.url, mode.sensitive);
    tracing::info!(target: "helix_wire_debug", method = %request.method, url = %url, wire_headers = %headers, wire_data = %data, "HTTP request");
}

pub(super) fn dump_http_response(status: u16, headers: &[(String, String)], data: &Bytes) {
    let Some(mode) = DumpMode::http() else {
        return;
    };
    let headers = visible_header_pairs(headers, mode.sensitive);
    let data = preview_payload(data, mode.sensitive);
    tracing::info!(target: "helix_wire_debug", status, wire_headers = %headers, wire_data = %data, "HTTP response");
}

pub(super) fn dump_ws_handshake(url: &str, headers: &BTreeMap<String, String>) {
    let Some(mode) = DumpMode::ws() else {
        return;
    };
    let headers = visible_headers(headers, mode.sensitive);
    let url = visible_url(url, mode.sensitive);
    tracing::info!(target: "helix_wire_debug", url = %url, wire_headers = %headers, "WS handshake");
}

pub(super) fn dump_ws_inbound(data: &Bytes) {
    let Some(mode) = DumpMode::ws() else {
        return;
    };
    let data = preview_payload(data, mode.sensitive);
    tracing::info!(target: "helix_wire_debug", wire_data = %data, "WS inbound push");
}

pub(super) fn dump_ws_outbound(data: &Bytes) {
    let Some(mode) = DumpMode::ws() else {
        return;
    };
    let data = preview_payload(data, mode.sensitive);
    tracing::info!(target: "helix_wire_debug", wire_data = %data, "WS outbound frame");
}

#[derive(Clone, Copy)]
struct DumpMode {
    sensitive: bool,
}

impl DumpMode {
    fn http() -> Option<Self> {
        Self::for_scope("HELIX_DUMP_HTTP")
    }

    fn ws() -> Option<Self> {
        if env_enabled("HELIX_DUMP_FRAMES") {
            return Some(Self {
                sensitive: env_enabled("HELIX_DUMP_NETWORK_SENSITIVE"),
            });
        }
        Self::for_scope("HELIX_DUMP_WS")
    }

    fn for_scope(scope: &str) -> Option<Self> {
        let enabled = env_enabled("HELIX_DUMP_NETWORK") || env_enabled(scope);
        enabled.then_some(Self {
            sensitive: env_enabled("HELIX_DUMP_NETWORK_SENSITIVE"),
        })
    }
}

fn env_enabled(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn visible_headers(headers: &BTreeMap<String, String>, sensitive: bool) -> String {
    let pairs: Vec<_> = headers
        .iter()
        .map(|(name, value)| (name.clone(), visible_header(name, value, sensitive)))
        .collect();
    serde_json::to_string(&pairs).unwrap_or_else(|_| "<unprintable>".to_string())
}

fn visible_header_pairs(headers: &[(String, String)], sensitive: bool) -> String {
    let pairs: Vec<_> = headers
        .iter()
        .map(|(name, value)| (name.clone(), visible_header(name, value, sensitive)))
        .collect();
    serde_json::to_string(&pairs).unwrap_or_else(|_| "<unprintable>".to_string())
}

fn visible_header(name: &str, value: &str, sensitive: bool) -> String {
    if sensitive || !is_sensitive_header(name) {
        value.to_string()
    } else {
        REDACTED.to_string()
    }
}

fn is_sensitive_header(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "token",
        "secret",
        "api-key",
        "apikey",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

fn preview_bytes(data: &Bytes) -> String {
    let shown = data.len().min(PREVIEW_LIMIT);
    let mut preview = String::from_utf8_lossy(&data[..shown]).into_owned();
    if shown < data.len() {
        preview.push_str(&format!("...<truncated {} bytes>", data.len() - shown));
    }
    preview
}

fn preview_payload(data: &Bytes, sensitive: bool) -> String {
    if sensitive {
        return preview_bytes(data);
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return format!("<redacted non-json {} bytes>", data.len());
    };
    redact_json(&mut value);
    preview_bytes(&Bytes::from(serde_json::to_vec(&value).unwrap_or_default()))
}

fn redact_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(fields) => {
            for (name, value) in fields {
                if is_sensitive_field(name) {
                    *value = serde_json::Value::String(REDACTED.to_string());
                } else {
                    redact_json(value);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_json),
        _ => {}
    }
}

fn is_sensitive_field(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    [
        "authorization",
        "cookie",
        "token",
        "secret",
        "password",
        "message",
        "text",
        "content",
        "body",
    ]
    .iter()
    .any(|needle| name.contains(needle))
}

fn visible_url(url: &str, sensitive: bool) -> String {
    if sensitive {
        return url.to_string();
    }
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let redacted_query = query
        .split('&')
        .map(|part| {
            part.split_once('=')
                .map_or(REDACTED.to_string(), |(key, _)| format!("{key}={REDACTED}"))
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted_query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_headers_are_redacted_by_default() {
        let headers = BTreeMap::from([
            ("cookieId".to_string(), "user-444".to_string()),
            ("Authorization".to_string(), "Bearer secret".to_string()),
            ("companyId".to_string(), "company-1".to_string()),
        ]);

        let visible = visible_headers(&headers, false);
        assert!(!visible.contains("user-444"));
        assert!(!visible.contains("Bearer secret"));
        assert!(visible.contains("company-1"));
    }

    #[test]
    fn sensitive_mode_explicitly_exposes_credentials() {
        let headers = BTreeMap::from([("cookieId".to_string(), "user-444".to_string())]);
        assert!(visible_headers(&headers, true).contains("user-444"));
    }

    #[test]
    fn payload_preview_is_bounded() {
        let data = Bytes::from(vec![b'x'; PREVIEW_LIMIT + 7]);
        let preview = preview_bytes(&data);
        assert!(preview.ends_with("<truncated 7 bytes>"));
    }

    #[test]
    fn default_payload_and_url_hide_credentials_and_message_content() {
        let data = Bytes::from_static(
            br#"{"action":"ping","message":"hello","data":{"token":"secret","seq":7}}"#,
        );
        let visible = preview_payload(&data, false);
        assert!(visible.contains("ping"));
        assert!(visible.contains("\"seq\":7"));
        assert!(!visible.contains("hello"));
        assert!(!visible.contains("secret"));

        let url = visible_url("http://host/path?cookieId=u1&token=t1", false);
        assert_eq!(url, "http://host/path?cookieId=<redacted>&token=<redacted>");
    }
}
