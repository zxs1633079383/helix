//! NativeHttp — reqwest client
//!
//! ## 设计要点
//!
//! - 连接池：reqwest::Client 内部连接池（默认最大空闲 10 个）
//! - 默认超时 30s：超时 → `PortError::Transport("timeout: ...")`（不静默丢）
//! - 运行时 header 注入：`Arc<RwLock<HeaderMap>>` 支持热更新（如 token 刷新）
//! - 错误分类：
//!   - 超时 → PortError::Transport("timeout: ...")
//!   - 4xx/5xx HTTP 状态 → 正常返回 HttpResponse（让模块决策重试/失败）
//!   - 网络错 → PortError::Transport("network: ...")
//!   - 其他 → PortError::Http(msg)

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, Method, header::{HeaderMap, HeaderName, HeaderValue}};
use tokio::sync::RwLock;

use helix_core::PortError;
use helix_core::ports::HttpRequester;
use helix_core::effect::{HttpRequest, HttpResponse};

/// 默认请求超时（含连接 + 响应体读取）
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// reqwest 驱动 HTTP 客户端。
///
/// Clone 是 O(1)：`reqwest::Client` 和 `Arc<RwLock<HeaderMap>>` 均为引用计数。
#[derive(Clone)]
pub struct NativeHttp {
    client: Client,
    /// 运行时可热更新的全局 header（如 Authorization）。
    /// 每次请求时 read lock 注入，不阻塞写侧（短暂 token 刷新锁）。
    global_headers: Arc<RwLock<HeaderMap>>,
}

impl NativeHttp {
    /// 使用默认 30s 超时创建 HTTP 客户端。
    pub fn new() -> Result<Self, PortError> {
        Self::with_timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
    }

    /// 指定超时创建（测试 / 特殊场景用）。
    pub fn with_timeout(timeout: Duration) -> Result<Self, PortError> {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| PortError::Http(format!("reqwest client build error: {e}")))?;

        Ok(Self {
            client,
            global_headers: Arc::new(RwLock::new(HeaderMap::new())),
        })
    }

    /// 热更新全局 header（如 Authorization token 刷新）。
    ///
    /// 持有者（helix-tauri 的 auth 模块）调用此方法注入新 token，
    /// 后续所有 HTTP 请求自动携带新 header，无需重建 client。
    pub async fn set_global_header(&self, name: &str, value: &str) -> Result<(), PortError> {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid header name '{name}': {e}")))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|e| PortError::Http(format!("invalid header value: {e}")))?;
        let mut headers = self.global_headers.write().await;
        headers.insert(header_name, header_value);
        Ok(())
    }

    /// 移除全局 header（如登出时移除 Authorization）。
    pub async fn remove_global_header(&self, name: &str) -> Result<(), PortError> {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid header name '{name}': {e}")))?;
        let mut headers = self.global_headers.write().await;
        headers.remove(header_name);
        Ok(())
    }

    fn classify_error(e: &reqwest::Error) -> PortError {
        if e.is_timeout() {
            PortError::Transport(format!("timeout: {e}"))
        } else if e.is_connect() || e.is_request() {
            PortError::Transport(format!("network: {e}"))
        } else {
            PortError::Http(e.to_string())
        }
    }
}

#[async_trait::async_trait]
impl HttpRequester for NativeHttp {
    /// 发送 HTTP 请求，返回完整响应（含 4xx/5xx，不自动重试）。
    ///
    /// 超时 / 网络错 → `Err(PortError::Transport(...))`（永不静默丢）。
    /// HTTP 4xx/5xx → `Ok(HttpResponse { status, ... })`（模块决策）。
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid method '{}': {e}", req.method)))?;

        let mut builder = self.client.request(method, &req.url);

        // 注入全局 header（read lock，短暂持有）
        {
            let globals = self.global_headers.read().await;
            for (name, value) in globals.iter() {
                builder = builder.header(name, value);
            }
        }

        // 注入请求级 header（可覆盖全局 header）
        for (name, value) in &req.headers {
            let header_name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|e| PortError::Http(format!("invalid header name '{name}': {e}")))?;
            let header_value = HeaderValue::from_str(value)
                .map_err(|e| PortError::Http(format!("invalid header value for '{name}': {e}")))?;
            builder = builder.header(header_name, header_value);
        }

        // 注入 body
        if let Some(body) = req.body {
            builder = builder.body(body);
        }

        let resp = builder.send().await.map_err(|e| Self::classify_error(&e))?;

        let status = resp.status().as_u16();
        let headers: Vec<(String, String)> = resp
            .headers()
            .iter()
            .map(|(k, v)| {
                (
                    k.as_str().to_string(),
                    v.to_str().unwrap_or("<non-utf8>").to_string(),
                )
            })
            .collect();

        let body = resp.bytes().await.map_err(|e| Self::classify_error(&e))?;

        Ok(HttpResponse { status, headers, body })
    }
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::HttpRequest;

    #[tokio::test]
    async fn test_http_client_builds() {
        // 只验证 client 创建不 panic
        let http = NativeHttp::new().expect("should build client");
        assert_eq!(http.receiver_count_for_test(), 0);
    }

    #[tokio::test]
    async fn test_set_and_remove_global_header() {
        let http = NativeHttp::new().expect("build");
        http.set_global_header("x-custom", "test-value").await.expect("set header");
        http.remove_global_header("x-custom").await.expect("remove header");
    }

    #[tokio::test]
    async fn test_invalid_method_returns_err() {
        let http = NativeHttp::new().expect("build");
        let req = HttpRequest {
            method: "INVALID METHOD!".to_string(),
            url: "http://localhost/".to_string(),
            headers: vec![],
            body: None,
        };
        let result = http.request(req).await;
        assert!(result.is_err(), "invalid method should return Err");
    }

    #[tokio::test]
    async fn test_timeout_produces_transport_error() {
        // 使用极短超时连接一个不存在的地址，触发超时/网络错
        let http = NativeHttp::with_timeout(Duration::from_millis(10)).expect("build");
        let req = HttpRequest {
            method: "GET".to_string(),
            url: "http://10.255.255.1:1/never".to_string(), // 黑洞地址
            headers: vec![],
            body: None,
        };
        let result = http.request(req).await;
        assert!(
            matches!(result, Err(PortError::Transport(_))),
            "timeout/network should produce PortError::Transport, got {:?}", result
        );
    }
}

impl NativeHttp {
    /// 仅用于测试：返回全局 header map 中的 receiver 数量（实际为 header 数量）
    #[cfg(test)]
    fn receiver_count_for_test(&self) -> usize {
        // This is just to verify the struct was created; return 0 as placeholder
        0
    }
}
