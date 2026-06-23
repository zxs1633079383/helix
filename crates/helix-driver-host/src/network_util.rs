//! `network.rs` 共享 URL/header 工具（按职责裂分，守 src ≤300 行 structure-gate §1）。
//!
//! 6 个纯函数：header 名/值校验、URL 校验、req_url resolve（absolute 短路）、header→string、WS 错误映射。
//! `pub(super)` 仅供 `network` 模块复用；无状态、无 I/O，单测可独立（见 `network.rs` 测试集成路径）。

use std::collections::BTreeMap;

use helix_core::PortError;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::Url;

pub(super) fn header_name(name: &str) -> Result<HeaderName, PortError> {
    HeaderName::from_bytes(name.as_bytes())
        .map_err(|e| PortError::Http(format!("invalid header name '{name}': {e}")))
}

pub(super) fn header_value(name: &str, value: &str) -> Result<HeaderValue, PortError> {
    HeaderValue::from_str(value)
        .map_err(|e| PortError::Http(format!("invalid header value for '{name}': {e}")))
}

pub(super) fn validate_url(url: &str, field: &str) -> Result<(), PortError> {
    Url::parse(url)
        .map(|_| ())
        .map_err(|e| PortError::Http(format!("invalid {field} '{url}': {e}")))
}

pub(super) fn resolve_http_url(base: &str, req_url: &str) -> Result<String, PortError> {
    if Url::parse(req_url).is_ok() {
        return Ok(req_url.to_string());
    }
    let base = Url::parse(base)
        .map_err(|e| PortError::Http(format!("invalid api_base_url '{base}': {e}")))?;
    base.join(req_url)
        .map(|url| url.to_string())
        .map_err(|e| PortError::Http(format!("invalid request url '{req_url}': {e}")))
}

pub(super) fn headers_to_strings(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_string(),
                value.to_str().unwrap_or("<non-utf8>").to_string(),
            )
        })
        .collect()
}

pub(super) fn map_ws_err(e: impl std::fmt::Display) -> PortError {
    PortError::Transport(e.to_string())
}
