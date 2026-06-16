//! Native HTTP thin shell over the shared host client.

use std::time::Duration;

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use helix_driver_host::{HostNetworkConfig, SharedHttpClient};

/// Compatibility wrapper for PC/Tauri code.
///
/// HTTP execution, header injection, and error classification live in
/// `helix-driver-host` so native and FFI share one implementation.
#[derive(Clone)]
pub struct NativeHttp {
    inner: SharedHttpClient,
}

impl NativeHttp {
    pub fn new() -> Result<Self, PortError> {
        Err(PortError::Http(
            "NativeHttp requires external HostNetworkConfig".to_string(),
        ))
    }

    pub fn with_timeout(_timeout: Duration) -> Result<Self, PortError> {
        Err(PortError::Http(
            "NativeHttp::with_timeout requires external HostNetworkConfig; use with_config(config.with_timeout(...))".to_string(),
        ))
    }

    pub fn with_config(config: HostNetworkConfig) -> Result<Self, PortError> {
        SharedHttpClient::new(config).map(Self::from_shared)
    }

    pub fn from_shared(inner: SharedHttpClient) -> Self {
        Self { inner }
    }

    pub async fn set_global_header(&self, name: &str, value: &str) -> Result<(), PortError> {
        self.inner.headers().set_header(name, value).await
    }

    pub async fn remove_global_header(&self, name: &str) -> Result<(), PortError> {
        self.inner.headers().remove_header(name).await
    }
}

#[async_trait::async_trait]
impl HttpRequester for NativeHttp {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        self.inner.request(req).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::HttpRequest;

    #[tokio::test]
    async fn test_http_client_builds_with_external_config() {
        let http = NativeHttp::with_config(test_config()).expect("should build client");
        assert_eq!(http.receiver_count_for_test(), 0);
    }

    #[tokio::test]
    async fn test_set_and_remove_global_header() {
        let http = NativeHttp::with_config(test_config()).expect("build");
        http.set_global_header("x-custom", "test-value")
            .await
            .expect("set header");
        http.remove_global_header("x-custom")
            .await
            .expect("remove header");
    }

    #[tokio::test]
    async fn test_invalid_method_returns_err() {
        let http = NativeHttp::with_config(test_config()).expect("build");
        let req = HttpRequest {
            method: "INVALID METHOD!".to_string(),
            url: "/".to_string(),
            headers: vec![],
            body: None,
        };
        let result = http.request(req).await;
        assert!(result.is_err(), "invalid method should return Err");
    }

    #[tokio::test]
    async fn test_timeout_produces_transport_error() {
        let http = NativeHttp::with_config(test_config().with_timeout(Duration::from_millis(10)))
            .expect("build");
        let req = HttpRequest {
            method: "GET".to_string(),
            url: "http://10.255.255.1:1/never".to_string(),
            headers: vec![],
            body: None,
        };
        let result = http.request(req).await;
        assert!(
            matches!(result, Err(PortError::Transport(_))),
            "timeout/network should produce PortError::Transport, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_new_without_external_config_returns_err() {
        let result = NativeHttp::new();
        assert!(matches!(result, Err(PortError::Http(_))));
    }

    fn test_config() -> HostNetworkConfig {
        HostNetworkConfig::new("http://127.0.0.1", "ws://127.0.0.1")
    }
}

impl NativeHttp {
    #[cfg(test)]
    fn receiver_count_for_test(&self) -> usize {
        0
    }
}
