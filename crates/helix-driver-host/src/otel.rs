use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::trace::{parse_trace_id, TraceCarrier};

mod exporter;

use exporter::OtlpExporter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostOtelConfig {
    pub enabled: bool,
    pub service_name: String,
    pub endpoint: String,
    pub protocol: String,
}

impl Default for HostOtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: "helix-driver-host".to_string(),
            endpoint: "http://opentelemetry-collector.monitoring.svc.cluster.local:4317"
                .to_string(),
            protocol: "grpc".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceDirection {
    Inbound,
    Outbound,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostSpanSnapshot {
    pub name: &'static str,
    pub direction: TraceDirection,
    pub trace_id: Option<String>,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub exported: bool,
}

#[derive(Clone, Debug)]
pub struct HostOtelRuntime {
    inner: Arc<RuntimeInner>,
}

#[derive(Debug)]
struct RuntimeInner {
    config: HostOtelConfig,
    last_span: Mutex<Option<HostSpanSnapshot>>,
    seq: AtomicU64,
    exporter: Option<Arc<OtlpExporter>>,
}

impl HostOtelRuntime {
    pub fn new(config: HostOtelConfig) -> Self {
        let exporter = if config.enabled
            && !config.endpoint.eq_ignore_ascii_case("noop")
            && !config.protocol.eq_ignore_ascii_case("noop")
        {
            OtlpExporter::new(&config).map(Arc::new)
        } else {
            None
        };

        Self {
            inner: Arc::new(RuntimeInner {
                config,
                last_span: Mutex::new(None),
                seq: AtomicU64::new(1),
                exporter,
            }),
        }
    }

    pub fn from_env(default_service_name: &str) -> Self {
        let defaults = HostOtelConfig::default();
        Self::new(HostOtelConfig {
            enabled: env_flag("HELIX_OTEL_ENABLED", defaults.enabled),
            service_name: std::env::var("HELIX_OTEL_SERVICE_NAME")
                .or_else(|_| std::env::var("OTEL_SERVICE_NAME"))
                .unwrap_or_else(|_| default_service_name.to_string()),
            endpoint: std::env::var("HELIX_OTEL_ENDPOINT")
                .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
                .unwrap_or(defaults.endpoint),
            protocol: std::env::var("HELIX_OTEL_PROTOCOL").unwrap_or(defaults.protocol),
        })
    }

    pub fn span(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
    ) -> HostSpanScope {
        self.span_with_attributes(name, direction, carrier, Vec::new())
    }

    pub fn span_with_attributes(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
        attributes: Vec<(&'static str, String)>,
    ) -> HostSpanScope {
        let parent_span_id = carrier
            .and_then(|value| value.traceparent.as_deref())
            .and_then(parse_parent_span_id);
        let mut span_id = self.next_span_id();
        if parent_span_id.as_deref() == Some(span_id.as_str()) {
            span_id = self.next_span_id();
        }
        let trace_id = carrier
            .and_then(|value| value.traceparent.as_deref())
            .and_then(parse_trace_id)
            .unwrap_or_else(|| self.next_trace_id());
        let snapshot = HostSpanSnapshot {
            name,
            direction,
            trace_id: Some(trace_id),
            span_id,
            parent_span_id,
            attributes: attributes
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
            exported: self.inner.exporter.is_some(),
        };

        if self.inner.config.enabled && self.inner.exporter.is_none() {
            *self.inner.last_span.lock() = Some(snapshot.clone());
        }

        HostSpanScope {
            runtime: self.clone(),
            snapshot: Some(snapshot),
            baggage: carrier.and_then(|value| value.baggage.clone()),
            start_time: SystemTime::now(),
        }
    }

    pub fn last_span_for_test(&self) -> Option<HostSpanSnapshot> {
        self.inner.last_span.lock().clone()
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.config.enabled
    }

    pub fn config(&self) -> &HostOtelConfig {
        &self.inner.config
    }

    pub fn dropped_span_count(&self) -> u64 {
        self.inner
            .exporter
            .as_ref()
            .map_or(0, |exporter| exporter.dropped_span_count())
    }

    fn next_span_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id() as u64;
        let id = mix_span_seed(nanos ^ seq.rotate_left(17) ^ pid.rotate_left(33));
        format!("{:016x}", id.max(1))
    }

    fn next_trace_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed) as u128;
        format!("{:032x}", nanos ^ seq)
    }

    fn export(
        &self,
        snapshot: HostSpanSnapshot,
        baggage: Option<String>,
        start_time: SystemTime,
        end_time: SystemTime,
    ) {
        let Some(exporter) = &self.inner.exporter else {
            return;
        };
        exporter.try_enqueue(snapshot, baggage, start_time, end_time);
    }
}

#[derive(Debug)]
pub struct HostSpanScope {
    runtime: HostOtelRuntime,
    snapshot: Option<HostSpanSnapshot>,
    baggage: Option<String>,
    start_time: SystemTime,
}

impl HostSpanScope {
    pub fn trace_id_for_test(&self) -> Option<String> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.trace_id.clone())
    }

    pub fn name_for_test(&self) -> &'static str {
        self.snapshot.as_ref().map_or("", |snapshot| snapshot.name)
    }
}

impl Drop for HostSpanScope {
    fn drop(&mut self) {
        let Some(snapshot) = self.snapshot.take() else {
            return;
        };
        self.runtime.export(
            snapshot,
            self.baggage.take(),
            self.start_time,
            SystemTime::now(),
        );
    }
}

fn mix_span_seed(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn parse_parent_span_id(traceparent: &str) -> Option<String> {
    if parse_trace_id(traceparent).is_none() {
        return None;
    }
    Some(traceparent[36..52].to_string())
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim(),
                "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(default)
}

#[cfg(test)]
mod otel_tests;
