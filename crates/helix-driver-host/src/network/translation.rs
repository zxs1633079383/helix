//! Isolated translation HTTP client.
//!
//! Translation requests deliberately bypass the IM `SharedHttpClient` header registry and
//! `CrossCuttingHttp`.  The endpoint and optional API-key header are host-owned configuration;
//! core never receives either value.

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use reqwest::header::CONTENT_TYPE;
use reqwest::{Client, Method, Url};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

use super::network_util::{header_name, header_value};

/// Host-owned configuration for the reserved `service:translation` route.
// The config may carry an API token.  Keep it cloneable for host assembly but do not derive
// `Debug`, because generic configuration logging must never print the credential value.
#[derive(Clone)]
pub struct TranslationHttpConfig {
    /// Explicit translation service URL.  It is never derived from the IM API base URL.
    pub endpoint: String,
    /// Per-request timeout.  The caller normally supplies 30 seconds.
    pub timeout: Duration,
    /// Optional dedicated credential, for example `("X-API-Key", "...")`.
    /// It is never copied from the main session/bot auth registry.
    pub auth_header: Option<(String, String)>,
}

impl TranslationHttpConfig {
    pub fn new(
        endpoint: impl Into<String>,
        timeout: Duration,
        auth_header: Option<(String, String)>,
    ) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout,
            auth_header,
        }
    }
}

/// Dedicated reqwest client for translation traffic.
#[derive(Clone)]
pub struct TranslationHttpClient {
    client: Client,
    endpoint: String,
    auth_header: Option<(reqwest::header::HeaderName, reqwest::header::HeaderValue)>,
    /// Translation traffic is intentionally capped below the shared IM HTTP limit.
    inflight: Arc<Semaphore>,
    request_guard: Option<crate::RequestAdmissionGuard>,
}

impl TranslationHttpClient {
    /// Opt-in host fact check; default adapters keep their previous network behavior.
    pub fn with_request_guard(mut self, guard: crate::RequestAdmissionGuard) -> Self {
        self.request_guard = Some(guard);
        self
    }

    pub fn new(config: TranslationHttpConfig) -> Result<Self, PortError> {
        let endpoint = Url::parse(&config.endpoint)
            .map_err(|error| PortError::Http(format!("invalid translation endpoint: {error}")))?;
        if endpoint.host_str().is_none() {
            return Err(PortError::Http(
                "translation endpoint must include a host".into(),
            ));
        }
        if !matches!(endpoint.scheme(), "http" | "https") {
            return Err(PortError::Http(format!(
                "translation endpoint has unsupported scheme '{}'",
                endpoint.scheme()
            )));
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Err(PortError::Http(
                "translation endpoint must not include userinfo".into(),
            ));
        }
        if endpoint.query().is_some() || endpoint.fragment().is_some() {
            return Err(PortError::Http(
                "translation endpoint must not include query or fragment".into(),
            ));
        }
        if config.timeout.is_zero() {
            return Err(PortError::Http(
                "translation timeout must be greater than zero".into(),
            ));
        }
        let auth_header = config
            .auth_header
            .map(|(name, value)| {
                if is_forbidden_config_header(&name) {
                    return Err(PortError::Http(
                        "translation auth must use a dedicated non-session header".into(),
                    ));
                }
                Ok((header_name(&name)?, header_value(name.as_str(), &value)?))
            })
            .transpose()?;
        let redirect_origin = endpoint.clone();
        let client = Client::builder()
            .timeout(config.timeout)
            // A dedicated credential may follow only same-origin redirects.  This keeps an
            // explicitly configured HTTP endpoint usable for private services while preventing
            // a redirect from forwarding the translation request to another host or port.
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt
                    .previous()
                    .first()
                    .is_some_and(|initial| same_origin(initial, attempt.url()))
                    && same_origin(&redirect_origin, attempt.url())
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(|error| PortError::Http(format!("translation client build error: {error}")))?;
        Ok(Self {
            client,
            endpoint: endpoint.to_string(),
            auth_header,
            inflight: Arc::new(Semaphore::new(2)),
            request_guard: None,
        })
    }

    async fn execute(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        // Cancellation removes only the core corr; the permit remains owned by this future until
        // the physical request finishes.  `try_acquire_owned` keeps cancelled-request storms from
        // queueing behind the two inference slots or consuming the shared HTTP worker pool.
        let _permit = Arc::clone(&self.inflight)
            .try_acquire_owned()
            .map_err(|_| PortError::Other("translation concurrency limit reached".into()))?;
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|error| PortError::Http(format!("invalid translation method: {error}")))?;
        let mut builder = self.client.request(method, &self.endpoint);
        builder = builder.header(CONTENT_TYPE, "application/json");
        if let Some((name, value)) = &self.auth_header {
            builder = builder.header(name, value);
        }
        for (name, value) in req.headers {
            // The reserved route only accepts safe per-request content headers.  In particular,
            // callers cannot smuggle the IM session identity into the isolated client.
            if is_forbidden_request_header(&name) {
                continue;
            }
            let name = header_name(&name)?;
            let value = header_value(name.as_str(), &value)?;
            builder = builder.header(name, value);
        }
        if let Some(body) = req.body {
            if body.len() > 16_000 {
                return Err(PortError::Other(
                    "translation request body exceeds 16000 bytes".into(),
                ));
            }
            builder = builder.body(body);
        }
        if let Some(guard) = &self.request_guard { guard()?; }
        let response = builder.send().await.map_err(classify_error)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or("<non-utf8>").to_string(),
                )
            })
            .collect();
        let body = response.bytes().await.map_err(classify_error)?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }
}

#[async_trait::async_trait]
impl HttpRequester for TranslationHttpClient {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        self.execute(req).await
    }
}

fn classify_error(error: reqwest::Error) -> PortError {
    if error.is_timeout() {
        PortError::Transport(format!("timeout: {error}"))
    } else if error.is_connect() || error.is_request() {
        PortError::Transport(format!("network: {error}"))
    } else {
        PortError::Http(error.to_string())
    }
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host() == right.host()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn is_forbidden_config_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("cookie")
        || name.eq_ignore_ascii_case("cookieId")
        || name.eq_ignore_ascii_case("userId")
        || name.eq_ignore_ascii_case("user-id")
        || name.eq_ignore_ascii_case("connectionId")
        || name.eq_ignore_ascii_case("connection-id")
        || name.eq_ignore_ascii_case(helix_core::AUTH_KIND_HEADER)
}

fn is_forbidden_request_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization") || is_forbidden_config_header(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::effect::HttpRequest;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::{mpsc, oneshot};

    #[test]
    fn explicit_http_and_https_translation_endpoints_are_allowed() {
        let local = TranslationHttpConfig::new(
            "http://127.0.0.1:18080/translate",
            Duration::from_secs(30),
            None,
        );
        assert!(TranslationHttpClient::new(local).is_ok());
        let private_http = TranslationHttpConfig::new(
            "http://translation.example/translate",
            Duration::from_secs(30),
            None,
        );
        assert!(TranslationHttpClient::new(private_http).is_ok());
        let public_https = TranslationHttpConfig::new(
            "https://translation.example/translate",
            Duration::from_secs(30),
            None,
        );
        assert!(TranslationHttpClient::new(public_https).is_ok());
    }

    #[test]
    fn translation_endpoint_requires_a_host() {
        let config =
            TranslationHttpConfig::new("mailto:translation", Duration::from_secs(30), None);
        assert!(TranslationHttpClient::new(config).is_err());
    }

    #[test]
    fn translation_endpoint_rejects_userinfo_query_and_fragment() {
        for endpoint in [
            "https://user:password@translation.example/translate",
            "https://translation.example/translate?tenant=secret",
            "https://translation.example/translate#fragment",
        ] {
            let config = TranslationHttpConfig::new(endpoint, Duration::from_secs(30), None);
            assert!(
                TranslationHttpClient::new(config).is_err(),
                "endpoint must reject unsafe URL component: {endpoint}"
            );
        }
    }

    #[test]
    fn internal_auth_intent_header_is_rejected_from_translation_config() {
        let config = TranslationHttpConfig::new(
            "http://127.0.0.1:18080/translate",
            Duration::from_secs(30),
            Some((helix_core::AUTH_KIND_HEADER.to_string(), "session".into())),
        );
        assert!(TranslationHttpClient::new(config).is_err());
    }

    #[test]
    fn session_identity_headers_are_rejected_from_translation_config() {
        for name in ["Cookie", "cookieId", "userId", "connectionId"] {
            let config = TranslationHttpConfig::new(
                "http://127.0.0.1:18080/translate",
                Duration::from_secs(30),
                Some((name.to_string(), "secret".into())),
            );
            assert!(
                TranslationHttpClient::new(config).is_err(),
                "session header {name} must not configure translation auth"
            );
        }
    }

    #[test]
    fn translation_inflight_limit_is_two_without_waiting_queue() {
        let config = TranslationHttpConfig::new(
            "http://127.0.0.1:18080/translate",
            Duration::from_secs(30),
            None,
        );
        let client = TranslationHttpClient::new(config).expect("loopback client builds");
        let first = Arc::clone(&client.inflight)
            .try_acquire_owned()
            .expect("first translation permit");
        let second = Arc::clone(&client.inflight)
            .try_acquire_owned()
            .expect("second translation permit");
        assert!(
            Arc::clone(&client.inflight).try_acquire_owned().is_err(),
            "third translation must fail fast while two physical requests remain active"
        );
        drop(first);
        assert!(Arc::clone(&client.inflight).try_acquire_owned().is_ok());
        drop(second);
    }

    #[tokio::test]
    async fn translation_cancel_storm_keeps_physical_permits_until_late_replies() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind delayed translation server");
        let address = listener.local_addr().expect("delayed translation address");
        let (started_tx, mut started_rx) = mpsc::channel::<()>(2);
        let (release_tx, release_rx) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let mut delayed_streams = Vec::with_capacity(2);
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().await.expect("accept translation");
                let mut request = [0_u8; 4096];
                let _ = stream
                    .read(&mut request)
                    .await
                    .expect("read translation request");
                started_tx
                    .send(())
                    .await
                    .expect("translation test receiver remains open");
                delayed_streams.push(stream);
            }
            let _ = release_rx.await;
            for mut stream in delayed_streams {
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\n\r\n{\"text\":\"ok\",\"from\":\"zh\",\"to\":\"en\"}",
                    )
                    .await
                    .expect("write delayed translation response");
            }
            let (mut stream, _) = listener.accept().await.expect("accept fresh translation");
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.expect("read fresh request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\n\r\n{\"text\":\"ok\",\"from\":\"zh\",\"to\":\"en\"}",
                )
                .await
                .expect("write fresh translation response");
        });

        let client = TranslationHttpClient::new(TranslationHttpConfig::new(
            format!("http://{address}/translate"),
            Duration::from_secs(5),
            None,
        ))
        .expect("loopback translation client builds");
        let request = || HttpRequest {
            method: "POST".into(),
            url: "service:translation".into(),
            headers: vec![],
            body: Some(Bytes::from_static(br#"{"text":"hello"}"#)),
        };
        let first = tokio::spawn({
            let client = client.clone();
            async move { client.request(request()).await }
        });
        let second = tokio::spawn({
            let client = client.clone();
            async move { client.request(request()).await }
        });
        for _ in 0..2 {
            tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
                .await
                .expect("two physical translation requests must start")
                .expect("translation server remains available");
        }

        // Core cancellation removes the corr entry, but has no CancelHttp port.  Keep these two
        // physical calls pending to model late replies, then flood replacements: none may queue
        // behind the two inference slots or exceed the transport lease bound.
        for _ in 0..32 {
            let result =
                tokio::time::timeout(Duration::from_millis(100), client.request(request()))
                    .await
                    .expect("cancel-storm replacement must fail without waiting");
            assert!(
                matches!(result, Err(PortError::Other(ref message)) if message.contains("concurrency")),
                "replacement must be rejected while cancelled physical calls are active: {result:?}"
            );
        }

        release_tx.send(()).expect("release delayed responses");
        assert_eq!(
            first
                .await
                .expect("join first translation")
                .expect("first reply")
                .status,
            200
        );
        assert_eq!(
            second
                .await
                .expect("join second translation")
                .expect("second reply")
                .status,
            200
        );
        // Once both late replies release their permits, a fresh request can acquire a slot again.
        let fresh = tokio::time::timeout(Duration::from_secs(1), client.request(request()))
            .await
            .expect("fresh translation must not remain blocked")
            .expect("fresh translation request");
        assert_eq!(fresh.status, 200);
        server.await.expect("join delayed translation server");
    }

    #[tokio::test]
    async fn translation_client_does_not_follow_cross_host_redirect() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redirect translation server");
        let address = listener.local_addr().expect("redirect translation address");
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept initial request");
            let mut request = [0_u8; 4096];
            let _ = stream
                .read(&mut request)
                .await
                .expect("read initial translation request");
            let location = format!("http://localhost:{}/translate", address.port());
            let response =
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n");
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write cross-host redirect");
        });

        let client = TranslationHttpClient::new(TranslationHttpConfig::new(
            format!("http://{address}/translate"),
            Duration::from_secs(5),
            None,
        ))
        .expect("redirect translation client builds");
        let response = tokio::time::timeout(
            Duration::from_secs(1),
            client.request(HttpRequest {
                method: "POST".into(),
                url: "service:translation".into(),
                headers: vec![],
                body: Some(Bytes::from_static(br#"{"text":"hello"}"#)),
            }),
        )
        .await
        .expect("cross-host redirect must return without following")
        .expect("initial redirect response should be returned");
        assert_eq!(response.status, 302);
        server.await.expect("join redirect translation server");
    }
}
