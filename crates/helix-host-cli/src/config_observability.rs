use serde::Deserialize;
use std::sync::Arc;

use helix_driver_host::{
    AsyncMetricSink, HostOtelConfig, HostOtelRuntime, MetricRuntimeConfig, NoopMetricSink,
};

/// 可观测性配置（主路径仍是 helix.config.yaml，不走 env 真源）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Observability {
    #[serde(default)]
    pub otel: Otel,
    #[serde(default)]
    pub metrics: Metrics,
}

impl Observability {
    /// YAML 是 host-cli OTel 的唯一生产真源；这里是 composition-root 适配点。
    pub fn otel_runtime(&self) -> HostOtelRuntime {
        self.otel_runtime_with_metrics(Arc::new(NoopMetricSink))
    }

    /// 用同一个有界 Metrics sink 装配 trace exporter 自观测，避免遥测故障成为盲区。
    pub fn otel_runtime_with_metrics(&self, metrics: Arc<dyn AsyncMetricSink>) -> HostOtelRuntime {
        let config = self.otel.host_config();
        tracing::info!(
            enabled = config.enabled,
            service = %config.service_name,
            protocol = %config.protocol,
            endpoint = %endpoint_for_log(&config.endpoint),
            "host-cli OTel 装配"
        );
        HostOtelRuntime::new_with_metric_sink_and_slow_threshold(
            config,
            metrics,
            std::time::Duration::from_millis(self.otel.slow_trace_threshold_ms),
        )
    }

    pub fn metrics_config(&self) -> MetricRuntimeConfig {
        self.metrics.host_config()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Metrics {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "serviceName", default = "d_helix_service_name")]
    pub service_name: String,
    #[serde(default = "d_otel_endpoint")]
    pub endpoint: String,
    #[serde(rename = "deploymentEnvironment", default = "d_local")]
    pub deployment_environment: String,
    #[serde(default = "d_native")]
    pub platform: String,
    #[serde(default = "d_canary")]
    pub scenario: String,
    #[serde(rename = "channelShape", default = "d_none")]
    pub channel_shape: String,
    #[serde(default = "d_baseline")]
    pub profile: String,
    #[serde(rename = "queueCapacity", default = "d_metrics_queue_capacity")]
    pub queue_capacity: usize,
    #[serde(rename = "batchMax", default = "d_metrics_batch_max")]
    pub batch_max: usize,
    #[serde(rename = "exportIntervalMs", default = "d_export_interval_ms")]
    pub export_interval_ms: u64,
    #[serde(rename = "exportTimeoutMs", default = "d_export_timeout_ms")]
    pub export_timeout_ms: u64,
    #[serde(rename = "shutdownTimeoutMs", default = "d_shutdown_timeout_ms")]
    pub shutdown_timeout_ms: u64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: d_helix_service_name(),
            endpoint: d_otel_endpoint(),
            deployment_environment: d_local(),
            platform: d_native(),
            scenario: d_canary(),
            channel_shape: d_none(),
            profile: d_baseline(),
            queue_capacity: d_metrics_queue_capacity(),
            batch_max: d_metrics_batch_max(),
            export_interval_ms: d_export_interval_ms(),
            export_timeout_ms: d_export_timeout_ms(),
            shutdown_timeout_ms: d_shutdown_timeout_ms(),
        }
    }
}

impl Metrics {
    pub fn host_config(&self) -> MetricRuntimeConfig {
        MetricRuntimeConfig {
            enabled: self.enabled,
            service_name: self.service_name.clone(),
            endpoint: self.endpoint.clone(),
            deployment_environment: self.deployment_environment.clone(),
            platform: self.platform.clone(),
            scenario: self.scenario.clone(),
            channel_shape: self.channel_shape.clone(),
            profile: self.profile.clone(),
            queue_capacity: self.queue_capacity,
            batch_max: self.batch_max,
            export_interval: std::time::Duration::from_millis(self.export_interval_ms),
            export_timeout: std::time::Duration::from_millis(self.export_timeout_ms),
            ..MetricRuntimeConfig::default()
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Otel {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "serviceName", default = "d_helix_service_name")]
    pub service_name: String,
    #[serde(default = "d_otel_endpoint")]
    pub endpoint: String,
    #[serde(default = "d_otel_protocol")]
    pub protocol: String,
    #[serde(rename = "slowTraceThresholdMs", default = "d_slow_trace_threshold_ms")]
    pub slow_trace_threshold_ms: u64,
    #[serde(default)]
    pub sampler: OtelSampler,
    #[serde(default)]
    pub facets: OtelFacets,
}

impl Default for Otel {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: d_helix_service_name(),
            endpoint: d_otel_endpoint(),
            protocol: d_otel_protocol(),
            slow_trace_threshold_ms: d_slow_trace_threshold_ms(),
            sampler: OtelSampler::default(),
            facets: OtelFacets::default(),
        }
    }
}

/// 默认 250ms 以上 Span 标记为慢 Trace，Grafana/Tempo 可直接过滤。
fn d_slow_trace_threshold_ms() -> u64 {
    250
}

impl Otel {
    pub fn host_config(&self) -> HostOtelConfig {
        HostOtelConfig {
            enabled: self.enabled,
            service_name: self.service_name.clone(),
            endpoint: self.endpoint.clone(),
            protocol: self.protocol.clone(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OtelSampler {
    #[serde(rename = "type", default = "d_sampler_type")]
    pub kind: String,
    #[serde(default = "d_sampler_ratio")]
    pub ratio: f64,
}

impl Default for OtelSampler {
    fn default() -> Self {
        Self {
            kind: d_sampler_type(),
            ratio: d_sampler_ratio(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OtelFacets {
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub core: bool,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub ws: bool,
    #[serde(default)]
    pub storage: bool,
    #[serde(rename = "portReply", default)]
    pub port_reply: bool,
    #[serde(default)]
    pub event: bool,
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub ffi: bool,
}

fn d_helix_service_name() -> String {
    "helix-host-cli".to_string()
}
fn d_otel_endpoint() -> String {
    "http://127.0.0.1:4317".to_string()
}
fn d_otel_protocol() -> String {
    "grpc".to_string()
}
fn d_sampler_type() -> String {
    "parentbased_traceidratio".to_string()
}
fn d_sampler_ratio() -> f64 {
    1.0
}
fn d_local() -> String {
    "local".to_string()
}
fn d_native() -> String {
    "native".to_string()
}
fn d_canary() -> String {
    "canary".to_string()
}
fn d_none() -> String {
    "none".to_string()
}
fn d_baseline() -> String {
    "baseline".to_string()
}
fn d_metrics_queue_capacity() -> usize {
    8192
}
fn d_metrics_batch_max() -> usize {
    512
}
fn d_export_interval_ms() -> u64 {
    5000
}
fn d_export_timeout_ms() -> u64 {
    2000
}
fn d_shutdown_timeout_ms() -> u64 {
    5000
}

pub(super) fn endpoint_for_log(endpoint: &str) -> String {
    let without_query = endpoint.split(['?', '#']).next().unwrap_or(endpoint);
    let Some((scheme, rest)) = without_query.split_once("://") else {
        return without_query.to_string();
    };
    let authority_and_path = rest.rsplit_once('@').map_or(rest, |(_, safe)| safe);
    format!("{scheme}://{authority_and_path}")
}
