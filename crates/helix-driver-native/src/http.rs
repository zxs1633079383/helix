//! Native HTTP thin shell over the shared host client.
//!
//! ## P1.5 出站横切层装配点
//!
//! HTTP 执行 / 鉴权头注入 / 错误分类的**实现**全在 `helix-driver-host`（native + FFI 共享，
//! ADR-007）。本 native 壳只做装配：
//! - 裸出站：[`NativeHttp::with_config`] / [`NativeHttp::from_shared`]（无横切，host-cli / 测试）。
//! - 带横切：[`NativeHttp::with_crosscut`] 把 `SharedHttpClient` 包进
//!   [`CrossCuttingHttp`]（鉴权意图→token 注入 + 业务 outcome observer +
//!   traceparent 透传）。**装配点就绪，泵接线待 M2**：当前
//!   host-cli 走裸 `Plain` 出站，横切语义未生效；M2 host 装配层（host-cli / Tauri shell）
//!   登录态就绪后改调 `with_crosscut` 接横切层（drift-review dbe1b74..7e9f3ca finding①）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use helix_driver_host::{
    AsyncMetricSink, AuthTokenRegistry, CrossCuttingHttp, HostHeaderRegistry, HostNetworkConfig,
    HostOtelRuntime, HttpOutcomeObserver, SharedHttpClient, TranslationHttpClient,
    TranslationHttpConfig,
};

use crate::event_sink::NativeEventSink;

/// 出站横切层具体类型（native 装配：内层 `SharedHttpClient` + `NativeEventSink` 回报通道）。
type NativeCrossHttp = CrossCuttingHttp<SharedHttpClient, NativeEventSink>;

/// Candidate-only one-shot failure switch; production callers never arm this test seam.
static WDIO_FAIL_NEXT_POST_CREATE: AtomicBool = AtomicBool::new(false);

/// Arm or clear the coarse-real candidate's next `/posts/create` failure.
pub fn configure_wdio_fail_next_post_create(armed: bool) {
    WDIO_FAIL_NEXT_POST_CREATE.store(armed, Ordering::SeqCst);
}

/// Consume the candidate fault only for the real post-create write, never for readback/media.
fn consume_wdio_post_create_fault(req: &HttpRequest) -> bool {
    if !req.method.eq_ignore_ascii_case("POST") || !req.url.contains("/posts/create") {
        return false;
    }
    WDIO_FAIL_NEXT_POST_CREATE
        .compare_exchange(true, false, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
}

/// Compatibility wrapper for PC/Tauri code.
///
/// HTTP execution, header injection, and error classification live in
/// `helix-driver-host` so native and FFI share one implementation. 横切层（鉴权/401/
/// outcome observer/trace）可选叠加——`with_crosscut` 装配后，请求经
/// [`CrossCuttingHttp`]，否则裸发。
///
/// `headers` 是底层 `SharedHttpClient` 的共享 registry 句柄（Arc clone），无论是否叠加横切层，
/// `set/remove_global_header`（connectionId / 通用头运行时注入）始终可达——横切层只管鉴权头，
/// 通用头仍走此 registry（不互相干扰）。
#[derive(Clone)]
pub struct NativeHttp {
    inner: HttpInner,
    headers: HostHeaderRegistry,
    translation: Option<TranslationHttpClient>,
}

#[derive(Clone)]
enum HttpInner {
    /// 裸出站（无横切；host-cli / 单测）。
    Plain(SharedHttpClient),
    /// 带横切（鉴权注入 + 401/断网 emit + traceparent 透传）。
    Cross(NativeCrossHttp),
}

impl NativeHttp {
    // 构造唯一入口 = with_config / from_shared / with_crosscut（ADR-010 共享 client）。
    // 旧 new()/with_timeout() 曾退化为永远 Err 的活墓碑——已删，误用变编译错（D2 drift 收敛）。
    pub fn with_config(config: HostNetworkConfig) -> Result<Self, PortError> {
        SharedHttpClient::new(config).map(Self::from_shared)
    }

    pub fn with_config_and_metric_sink(
        config: HostNetworkConfig,
        metrics: Arc<dyn AsyncMetricSink>,
    ) -> Result<Self, PortError> {
        SharedHttpClient::new(config)
            .map(|client| client.with_metric_sink(metrics))
            .map(Self::from_shared)
    }

    pub fn from_shared(inner: SharedHttpClient) -> Self {
        let headers = inner.headers();
        Self {
            inner: HttpInner::Plain(inner),
            headers,
            translation: None,
        }
    }

    /// 装配出站横切层（P1.5 / M2 接缝③）：把裸 `SharedHttpClient` 包进 [`CrossCuttingHttp`]，
    /// 注入业务事件回报通道（`event_sink`）+ 真鉴权 token 源（`auth`，平台壳登录后写入）+
    /// 业务结果策略（`outcome_observer`）。
    /// 底层 registry 句柄保留 → 通用头注入路径不受横切层影响。
    pub fn with_crosscut(
        client: SharedHttpClient,
        event_sink: Arc<NativeEventSink>,
        auth: AuthTokenRegistry,
        outcome_observer: HttpOutcomeObserver,
    ) -> Self {
        let headers = client.headers();
        Self {
            inner: HttpInner::Cross(CrossCuttingHttp::new(
                client,
                event_sink,
                auth,
                outcome_observer,
            )),
            headers,
            translation: None,
        }
    }

    /// host-cli/Tauri composition root 显式注入 OTel；兼容构造仍保持 no-op。
    pub fn with_crosscut_and_otel(
        client: SharedHttpClient,
        event_sink: Arc<NativeEventSink>,
        auth: AuthTokenRegistry,
        outcome_observer: HttpOutcomeObserver,
        otel: HostOtelRuntime,
    ) -> Self {
        let headers = client.headers();
        Self {
            inner: HttpInner::Cross(
                CrossCuttingHttp::new(client, event_sink, auth, outcome_observer).with_otel(otel),
            ),
            headers,
            translation: None,
        }
    }

    /// Attach a dedicated translation client.  Its endpoint, credential and header registry are
    /// isolated from the main IM client; the reserved `service:translation` URL is the only route
    /// that can reach it.
    pub fn with_translation(
        mut self,
        config: TranslationHttpConfig,
    ) -> Result<Self, PortError> {
        self.translation = Some(TranslationHttpClient::new(config)?);
        Ok(self)
    }

    pub async fn set_global_header(&self, name: &str, value: &str) -> Result<(), PortError> {
        self.headers.set_header(name, value).await
    }

    pub async fn remove_global_header(&self, name: &str) -> Result<(), PortError> {
        self.headers.remove_header(name).await
    }

    pub fn is_otel_enabled(&self) -> bool {
        match &self.inner {
            HttpInner::Plain(_) => false,
            HttpInner::Cross(inner) => inner.is_otel_enabled(),
        }
    }
}

#[async_trait::async_trait]
impl HttpRequester for NativeHttp {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        if consume_wdio_post_create_fault(&req) {
            return Err(PortError::Transport(
                "wdio coarse-real injected posts/create failure".to_string(),
            ));
        }
        if req.url == "service:translation" {
            return self
                .translation
                .as_ref()
                .ok_or_else(|| {
                    PortError::Other("translation endpoint is not configured".into())
                })?
                .request(req)
                .await;
        }
        match &self.inner {
            HttpInner::Plain(c) => c.request(req).await,
            HttpInner::Cross(c) => c.request(req).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::HttpRequest;
    use std::time::Duration;

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
    async fn translation_route_is_explicitly_disabled_without_dedicated_config() {
        let http = NativeHttp::with_config(test_config()).expect("build client");
        let req = HttpRequest {
            method: "POST".to_string(),
            url: "service:translation".to_string(),
            headers: vec![],
            body: Some(bytes::Bytes::from_static(br#"{}"#)),
        };
        assert!(matches!(
            http.request(req).await,
            Err(PortError::Other(message)) if message.contains("translation endpoint is not configured")
        ));
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

    /// P1.5 装配 smoke：with_crosscut 叠加横切层后，通用头注入路径仍可达（registry 句柄保留），
    /// 且断网请求经横切层产 Transport 错误（横切语义详测在 host `p1_5_http_crosscut_test.rs`）。
    #[tokio::test]
    async fn test_with_crosscut_keeps_header_path_and_transport_error() {
        use crate::event_sink::NativeEventSink;
        use helix_driver_host::AuthTokenRegistry;
        use std::sync::Arc;

        let client = SharedHttpClient::new(test_config().with_timeout(Duration::from_millis(10)))
            .expect("build client");
        let (sink, _rx) = NativeEventSink::new();
        let auth = AuthTokenRegistry::new();
        auth.set_session("cookieId", "u1");
        let http = NativeHttp::with_crosscut(client, Arc::new(sink), auth, |_, _| None);
        assert!(!http.is_otel_enabled(), "兼容入口必须保持真 no-op");

        // 通用头注入仍可达（横切叠加后 registry 句柄保留，不再 unreachable）。
        http.set_global_header("connectionId", "conn-1")
            .await
            .expect("set header on crosscut http");
        http.remove_global_header("connectionId")
            .await
            .expect("remove header on crosscut http");

        // 断网走横切层 → Transport 错误原样上抛。
        let req = HttpRequest {
            method: "GET".to_string(),
            url: "http://10.255.255.1:1/never".to_string(),
            headers: vec![],
            body: None,
        };
        assert!(matches!(
            http.request(req).await,
            Err(PortError::Transport(_))
        ));
    }

    #[test]
    fn explicit_crosscut_constructor_installs_enabled_otel() {
        use helix_driver_host::{HostOtelConfig, HostOtelRuntime};

        let client = SharedHttpClient::new(test_config()).expect("build client");
        let (sink, _rx) = NativeEventSink::new();
        let runtime = HostOtelRuntime::new(HostOtelConfig {
            enabled: true,
            service_name: "native-http-test".to_string(),
            endpoint: "noop".to_string(),
            protocol: "noop".to_string(),
            deployment_environment: "local".to_string(),
        });
        let http = NativeHttp::with_crosscut_and_otel(
            client,
            Arc::new(sink),
            AuthTokenRegistry::new(),
            |_, _| None,
            runtime,
        );
        assert!(http.is_otel_enabled());
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
