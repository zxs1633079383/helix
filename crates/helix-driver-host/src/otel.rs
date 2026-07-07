use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter, SpanLinks};
use opentelemetry_sdk::Resource;
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
    pub span_id: String,
    pub parent_span_id: Option<String>,
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

#[derive(Debug)]
struct OtlpExporter {
    exporter: Arc<opentelemetry_otlp::SpanExporter>,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    scope: InstrumentationScope,
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
        Self::new(HostOtelConfig {
            enabled: env_flag("HELIX_OTEL_ENABLED", true),
            service_name: std::env::var("HELIX_OTEL_SERVICE_NAME")
                .or_else(|_| std::env::var("OTEL_SERVICE_NAME"))
                .unwrap_or_else(|_| default_service_name.to_string()),
            endpoint: std::env::var("HELIX_OTEL_ENDPOINT")
                .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
                .unwrap_or_else(|_| {
                    "http://opentelemetry-collector.monitoring.svc.cluster.local:4317".to_string()
                }),
            protocol: std::env::var("HELIX_OTEL_PROTOCOL").unwrap_or_else(|_| "grpc".to_string()),
        })
    }

    pub fn span(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
    ) -> HostSpanScope {
        let span_id = self.next_span_id();
        let parent_span_id = carrier
            .and_then(|value| value.traceparent.as_deref())
            .and_then(parse_parent_span_id);
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
            exported: self.inner.exporter.is_some(),
        };

        if self.inner.config.enabled {
            *self.inner.last_span.lock() = Some(snapshot.clone());
        }

        HostSpanScope {
            runtime: self.clone(),
            snapshot,
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

    fn next_span_id(&self) -> String {
        format!("{:016x}", self.inner.seq.fetch_add(1, Ordering::Relaxed))
    }

    fn next_trace_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed) as u128;
        format!("{:032x}", nanos ^ seq)
    }

    fn export(&self, snapshot: HostSpanSnapshot, baggage: Option<String>, start_time: SystemTime) {
        let Some(exporter) = &self.inner.exporter else {
            return;
        };
        let Some(span) =
            span_data_from_snapshot(&snapshot, baggage, start_time, exporter.scope.clone())
        else {
            return;
        };
        let span_exporter = Arc::clone(&exporter.exporter);
        if let Some(runtime) = exporter.runtime.lock().as_ref() {
            runtime.spawn(async move {
                if let Err(error) = span_exporter.export(vec![span]).await {
                    tracing::warn!(error = %error, "helix otel span export failed");
                }
            });
        }
    }
}

#[derive(Debug)]
pub struct HostSpanScope {
    runtime: HostOtelRuntime,
    snapshot: HostSpanSnapshot,
    baggage: Option<String>,
    start_time: SystemTime,
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
        self.runtime
            .export(self.snapshot.clone(), self.baggage.clone(), self.start_time);
    }
}

impl OtlpExporter {
    fn new(config: &HostOtelConfig) -> Option<Self> {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("helix-otel-export")
            .worker_threads(1)
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                tracing::warn!(error = %error, "helix otel exporter disabled: runtime init failed");
                return None;
            }
        };

        let endpoint = config.endpoint.clone();
        let init_result = match std::thread::Builder::new()
            .name("helix-otel-init".to_string())
            .spawn(move || {
                let init_runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                init_runtime.block_on(async move {
                    opentelemetry_otlp::SpanExporter::builder()
                        .with_tonic()
                        .with_endpoint(endpoint)
                        .with_timeout(Duration::from_secs(5))
                        .build()
                        .map_err(|error| error.to_string())
                })
            }) {
            Ok(handle) => handle
                .join()
                .unwrap_or_else(|_| Err("otel exporter init thread panicked".to_string())),
            Err(error) => Err(error.to_string()),
        };

        let mut exporter = match init_result {
            Ok(exporter) => exporter,
            Err(error) => {
                tracing::warn!(error = %error, "helix otel exporter disabled: exporter init failed");
                return None;
            }
        };

        let resource = Resource::builder()
            .with_service_name(config.service_name.clone())
            .build();
        exporter.set_resource(&resource);

        Some(Self {
            exporter: Arc::new(exporter),
            runtime: Mutex::new(Some(runtime)),
            scope: InstrumentationScope::builder(config.service_name.clone()).build(),
        })
    }
}

impl Drop for OtlpExporter {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.lock().take() {
            runtime.shutdown_background();
        }
    }
}

fn span_data_from_snapshot(
    snapshot: &HostSpanSnapshot,
    baggage: Option<String>,
    start_time: SystemTime,
    scope: InstrumentationScope,
) -> Option<SpanData> {
    let trace_id = TraceId::from_hex(snapshot.trace_id.as_deref()?).ok()?;
    let span_id = SpanId::from_hex(&snapshot.span_id).ok()?;
    let parent_span_id = snapshot
        .parent_span_id
        .as_deref()
        .and_then(|id| SpanId::from_hex(id).ok())
        .unwrap_or(SpanId::INVALID);
    let end_time = SystemTime::now();
    let mut attributes = vec![
        KeyValue::new("service.layer", "helix"),
        KeyValue::new("span.direction", format!("{:?}", snapshot.direction)),
    ];
    if let Some(baggage) = baggage {
        attributes.push(KeyValue::new("baggage", baggage));
    }

    Some(SpanData {
        span_context: SpanContext::new(
            trace_id,
            span_id,
            TraceFlags::SAMPLED,
            false,
            TraceState::NONE,
        ),
        parent_span_id,
        parent_span_is_remote: parent_span_id != SpanId::INVALID,
        span_kind: span_kind_for_direction(snapshot.direction),
        name: snapshot.name.into(),
        start_time,
        end_time,
        attributes,
        dropped_attributes_count: 0,
        events: SpanEvents::default(),
        links: SpanLinks::default(),
        status: Status::Ok,
        instrumentation_scope: scope,
    })
}

fn span_kind_for_direction(direction: TraceDirection) -> SpanKind {
    match direction {
        TraceDirection::Inbound => SpanKind::Consumer,
        TraceDirection::Outbound => SpanKind::Producer,
        TraceDirection::Internal => SpanKind::Internal,
    }
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
