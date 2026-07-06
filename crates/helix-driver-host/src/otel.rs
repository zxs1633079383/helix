use std::sync::Arc;

use parking_lot::Mutex;

use crate::trace::{parse_trace_id, TraceCarrier};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostOtelConfig {
    pub enabled: bool,
    pub service_name: String,
    pub endpoint: String,
    pub protocol: String,
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
}

#[derive(Clone, Debug)]
pub struct HostOtelRuntime {
    inner: Arc<RuntimeInner>,
}

#[derive(Debug)]
struct RuntimeInner {
    config: HostOtelConfig,
    last_span: Mutex<Option<HostSpanSnapshot>>,
}

impl HostOtelRuntime {
    pub fn new(config: HostOtelConfig) -> Self {
        Self {
            inner: Arc::new(RuntimeInner {
                config,
                last_span: Mutex::new(None),
            }),
        }
    }

    pub fn from_env(default_service_name: &str) -> Self {
        Self::new(HostOtelConfig {
            enabled: env_flag("HELIX_OTEL_ENABLED"),
            service_name: std::env::var("HELIX_OTEL_SERVICE_NAME")
                .unwrap_or_else(|_| default_service_name.to_string()),
            endpoint: std::env::var("HELIX_OTEL_ENDPOINT").unwrap_or_else(|_| "noop".to_string()),
            protocol: std::env::var("HELIX_OTEL_PROTOCOL").unwrap_or_else(|_| "noop".to_string()),
        })
    }

    pub fn span(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
    ) -> HostSpanScope {
        let snapshot = HostSpanSnapshot {
            name,
            direction,
            trace_id: carrier
                .and_then(|value| value.traceparent.as_deref())
                .and_then(parse_trace_id),
        };

        if self.inner.config.enabled {
            *self.inner.last_span.lock() = Some(snapshot.clone());
        }

        HostSpanScope {
            snapshot,
            enabled: self.inner.config.enabled,
        }
    }

    pub fn last_span_for_test(&self) -> Option<HostSpanSnapshot> {
        self.inner.last_span.lock().clone()
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.config.enabled
    }
}

#[derive(Debug)]
pub struct HostSpanScope {
    snapshot: HostSpanSnapshot,
    enabled: bool,
}

impl HostSpanScope {
    pub fn trace_id_for_test(&self) -> Option<String> {
        self.snapshot.trace_id.clone()
    }

    pub fn name_for_test(&self) -> &'static str {
        self.snapshot.name
    }
}

impl Drop for HostSpanScope {
    fn drop(&mut self) {
        let _ = self.enabled;
    }
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim(),
                "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(false)
}
