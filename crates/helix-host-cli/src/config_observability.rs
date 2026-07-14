use serde::Deserialize;

use helix_driver_host::{HostOtelConfig, HostOtelRuntime};

/// 可观测性配置（主路径仍是 helix.config.yaml，不走 env 真源）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Observability {
    #[serde(default)]
    pub otel: Otel,
}

impl Observability {
    /// YAML 是 host-cli OTel 的唯一生产真源；这里是 composition-root 适配点。
    pub fn otel_runtime(&self) -> HostOtelRuntime {
        let config = self.otel.host_config();
        tracing::info!(
            enabled = config.enabled,
            service = %config.service_name,
            protocol = %config.protocol,
            endpoint = %endpoint_for_log(&config.endpoint),
            "host-cli OTel 装配"
        );
        HostOtelRuntime::new(config)
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
            sampler: OtelSampler::default(),
            facets: OtelFacets::default(),
        }
    }
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

pub(super) fn endpoint_for_log(endpoint: &str) -> String {
    let without_query = endpoint.split(['?', '#']).next().unwrap_or(endpoint);
    let Some((scheme, rest)) = without_query.split_once("://") else {
        return without_query.to_string();
    };
    let authority_and_path = rest.rsplit_once('@').map_or(rest, |(_, safe)| safe);
    format!("{scheme}://{authority_and_path}")
}
