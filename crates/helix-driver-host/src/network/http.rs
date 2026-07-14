use bytes::Bytes;
use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use reqwest::header::USER_AGENT;
use reqwest::{Client, Method};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::metrics::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

use super::network_util::{
    header_name, header_value, headers_to_strings, resolve_http_url, validate_url,
};
use super::{HostHeaderRegistry, HostNetworkConfig};

/// Request shape after shared config/header resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedHttpRequest {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Bytes>,
}

/// Shared reqwest-backed HTTP port implementation.
#[derive(Clone)]
pub struct SharedHttpClient {
    client: Client,
    config: HostNetworkConfig,
    headers: HostHeaderRegistry,
    metrics: Arc<dyn AsyncMetricSink>,
    inflight: Arc<AtomicUsize>,
}

impl SharedHttpClient {
    pub fn new(config: HostNetworkConfig) -> Result<Self, PortError> {
        Self::with_registry(config, HostHeaderRegistry::default())
    }

    pub fn with_registry(
        config: HostNetworkConfig,
        headers: HostHeaderRegistry,
    ) -> Result<Self, PortError> {
        validate_url(&config.api_base_url, "api_base_url")?;
        if !config.default_api_base_url.is_empty() {
            validate_url(&config.default_api_base_url, "default_api_base_url")?;
        }
        let mut builder = Client::builder().timeout(config.timeout);
        if let Some(user_agent) = &config.user_agent {
            builder = builder.user_agent(user_agent);
        }
        let client = builder
            .build()
            .map_err(|e| PortError::Http(format!("reqwest client build error: {e}")))?;
        Ok(Self {
            client,
            config,
            headers,
            metrics: Arc::new(NoopMetricSink),
            inflight: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn headers(&self) -> HostHeaderRegistry {
        self.headers.clone()
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }

    pub async fn prepare_for_test(
        &self,
        req: HttpRequest,
    ) -> Result<PreparedHttpRequest, PortError> {
        self.prepare(req).await
    }

    async fn prepare(&self, req: HttpRequest) -> Result<PreparedHttpRequest, PortError> {
        let method = Method::from_bytes(req.method.as_bytes())
            .map_err(|e| PortError::Http(format!("invalid method '{}': {e}", req.method)))?;
        let url = resolve_http_url(&self.config.api_base_url, &req.url)?;
        let mut headers = self.headers.snapshot().await;
        if let Some(user_agent) = &self.config.user_agent {
            headers.insert(USER_AGENT, header_value("user-agent", user_agent)?);
        }
        for (name, value) in &req.headers {
            let name = header_name(name)?;
            let value = header_value(name.as_str(), value)?;
            headers.insert(name, value);
        }
        Ok(PreparedHttpRequest {
            method: method.as_str().to_string(),
            url,
            headers: headers_to_strings(&headers),
            body: req.body,
        })
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
impl HttpRequester for SharedHttpClient {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        let started = Instant::now();
        let inflight = self.inflight.fetch_add(1, Ordering::Relaxed) + 1;
        record_http_gauge(self.metrics.as_ref(), inflight);
        let result = async {
            let prepared = self.prepare(req).await?;
            crate::network_debug::dump_http_request(&prepared);
            let method = Method::from_bytes(prepared.method.as_bytes()).map_err(|e| {
                PortError::Http(format!("invalid method '{}': {e}", prepared.method))
            })?;
            let mut builder = self.client.request(method, &prepared.url);
            for (name, value) in &prepared.headers {
                builder = builder.header(name, value);
            }
            if let Some(body) = prepared.body {
                builder = builder.body(body);
            }
            let resp = builder.send().await.map_err(|e| Self::classify_error(&e))?;
            let status = resp.status().as_u16();
            let headers = resp
                .headers()
                .iter()
                .map(|(k, v)| {
                    (
                        k.as_str().to_string(),
                        v.to_str().unwrap_or("<non-utf8>").to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let body = resp.bytes().await.map_err(|e| Self::classify_error(&e))?;
            crate::network_debug::dump_http_response(status, &headers, &body);
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        }
        .await;
        let remaining = self
            .inflight
            .fetch_sub(1, Ordering::Relaxed)
            .saturating_sub(1);
        record_http_gauge(self.metrics.as_ref(), remaining);
        let status = if result.is_ok() { "ok" } else { "error" };
        let labels = MetricLabels::one(LabelKey::Stage, "http_request")
            .with(LabelKey::Protocol, "http")
            .with(LabelKey::Operation, "request")
            .with(LabelKey::Status, status);
        let _ = self.metrics.try_record(MetricEvent::histogram(
            MetricId::HttpRequestDurationSeconds,
            started.elapsed().as_secs_f64(),
            labels,
        ));
        if result.is_err() {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::ErrorsTotal,
                1.0,
                labels.with(LabelKey::ErrorKind, "http_request_failed"),
            ));
        }
        result
    }
}

fn record_http_gauge(metrics: &dyn AsyncMetricSink, inflight: usize) {
    let _ = metrics.try_record(MetricEvent::gauge(
        MetricId::HttpInflight,
        inflight as f64,
        MetricLabels::one(LabelKey::Stage, "http_request"),
    ));
}
