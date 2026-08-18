use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use std::time::{Duration, SystemTime};

use opentelemetry::trace::{
    SpanContext, SpanId, SpanKind, Status, TraceFlags, TraceId, TraceState,
};
use opentelemetry::{InstrumentationScope, KeyValue};
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::{SpanData, SpanEvents, SpanExporter, SpanLinks};
use opentelemetry_sdk::Resource;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::{HostOtelConfig, HostSpanSnapshot, TraceDirection};
use crate::metrics::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};

const EXPORT_QUEUE_CAPACITY: usize = 1024;
const EXPORT_BATCH_SIZE: usize = 64;
const EXPORT_BATCH_DELAY: Duration = Duration::from_millis(200);

#[derive(Debug)]
struct QueuedSpan {
    snapshot: HostSpanSnapshot,
    baggage: Option<String>,
    start_time: SystemTime,
    end_time: SystemTime,
}

pub(super) struct OtlpExporter {
    sender: Option<mpsc::Sender<QueuedSpan>>,
    dropped_spans: AtomicU64,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    metrics: Arc<dyn AsyncMetricSink>,
}

impl OtlpExporter {
    /// 构造独立 trace worker，并发布固定 queue capacity 与 running 状态。
    pub(super) fn new(config: &HostOtelConfig, metrics: Arc<dyn AsyncMetricSink>) -> Option<Self> {
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

        let exporter = init_exporter(&runtime, config)?;
        let scope = InstrumentationScope::builder(config.service_name.clone()).build();
        let (sender, receiver) = mpsc::channel(EXPORT_QUEUE_CAPACITY);
        runtime.spawn(run_export_worker(
            receiver,
            exporter,
            scope,
            Arc::clone(&metrics),
        ));

        record_gauge(
            metrics.as_ref(),
            MetricId::TracesQueueCapacity,
            EXPORT_QUEUE_CAPACITY as f64,
        );
        record_gauge(metrics.as_ref(), MetricId::TracesQueueDepth, 0.0);
        record_gauge(metrics.as_ref(), MetricId::TracesExporterState, 1.0);

        Some(Self {
            sender: Some(sender),
            dropped_spans: AtomicU64::new(0),
            runtime: Mutex::new(Some(runtime)),
            metrics,
        })
    }

    pub(super) fn try_enqueue(
        &self,
        snapshot: HostSpanSnapshot,
        baggage: Option<String>,
        start_time: SystemTime,
        end_time: SystemTime,
    ) {
        let slow = snapshot
            .attributes
            .iter()
            .any(|(key, value)| key == "helix.slow" && value == "true");
        let queued = QueuedSpan {
            snapshot,
            baggage,
            start_time,
            end_time,
        };
        record_counter(self.metrics.as_ref(), MetricId::SpansCreatedTotal, 1.0);
        record_counter(self.metrics.as_ref(), MetricId::SpansSampledTotal, 1.0);
        if slow {
            record_counter(self.metrics.as_ref(), MetricId::SlowSpansTotal, 1.0);
        }
        let accepted = self.sender.as_ref().is_some_and(|sender| {
            let accepted = sender.try_send(queued).is_ok();
            if accepted {
                record_gauge(
                    self.metrics.as_ref(),
                    MetricId::TracesQueueDepth,
                    (sender.max_capacity() - sender.capacity()) as f64,
                );
            }
            accepted
        });
        if !accepted {
            self.dropped_spans.fetch_add(1, Ordering::Relaxed);
            record_counter(self.metrics.as_ref(), MetricId::TracesDroppedTotal, 1.0);
        }
    }

    pub(super) fn dropped_span_count(&self) -> u64 {
        self.dropped_spans.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for OtlpExporter {
    /// Debug 只暴露健康统计，绝不打印 endpoint、headers 或 exporter 内部状态。
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OtlpExporter")
            .field("dropped_spans", &self.dropped_span_count())
            .finish_non_exhaustive()
    }
}

impl Drop for OtlpExporter {
    fn drop(&mut self) {
        record_gauge(self.metrics.as_ref(), MetricId::TracesExporterState, 0.0);
        drop(self.sender.take());
        if let Some(runtime) = self.runtime.lock().take() {
            if tokio::runtime::Handle::try_current().is_ok() {
                let _ = std::thread::Builder::new()
                    .name("helix-otel-shutdown".to_string())
                    .spawn(move || runtime.shutdown_timeout(Duration::from_secs(5)))
                    .and_then(|handle| {
                        handle
                            .join()
                            .map_err(|_| std::io::Error::other("helix otel shutdown panicked"))
                    });
            } else {
                runtime.shutdown_timeout(Duration::from_secs(5));
            }
        }
    }
}

fn init_exporter(
    runtime: &tokio::runtime::Runtime,
    config: &HostOtelConfig,
) -> Option<opentelemetry_otlp::SpanExporter> {
    let endpoint = config.endpoint.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    runtime.spawn(async move {
        let result = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(Duration::from_secs(5))
            .build()
            .map_err(|error| error.to_string());
        let _ = tx.send(result);
    });
    let init_result = rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_else(|error| Err(error.to_string()));

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
    Some(exporter)
}

async fn run_export_worker(
    mut receiver: mpsc::Receiver<QueuedSpan>,
    exporter: opentelemetry_otlp::SpanExporter,
    scope: InstrumentationScope,
    metrics: Arc<dyn AsyncMetricSink>,
) {
    let mut batch = Vec::with_capacity(EXPORT_BATCH_SIZE);
    let mut export_success_logged = false;
    let mut health_tick = tokio::time::interval(Duration::from_secs(5));
    health_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_success = None;
    loop {
        let first = tokio::select! {
            queued = receiver.recv() => {
                let Some(queued) = queued else {
                    break;
                };
                queued
            }
            _ = health_tick.tick() => {
                if let Some(last_success) = last_success {
                    record_gauge(
                        metrics.as_ref(),
                        MetricId::TracesLastSuccessAgeSeconds,
                        Instant::now().duration_since(last_success).as_secs_f64(),
                    );
                }
                continue;
            }
        };
        record_gauge(
            metrics.as_ref(),
            MetricId::TracesQueueDepth,
            receiver.len() as f64,
        );
        push_span(&mut batch, first, &scope);
        let flush_deadline = tokio::time::sleep(EXPORT_BATCH_DELAY);
        tokio::pin!(flush_deadline);

        while batch.len() < EXPORT_BATCH_SIZE {
            tokio::select! {
                queued = receiver.recv() => {
                    let Some(queued) = queued else {
                        break;
                    };
                    push_span(&mut batch, queued, &scope);
                    record_gauge(
                        metrics.as_ref(),
                        MetricId::TracesQueueDepth,
                        receiver.len() as f64,
                    );
                }
                _ = &mut flush_deadline => break,
            }
        }

        if export_batch(
            &exporter,
            &mut batch,
            metrics.as_ref(),
            &mut export_success_logged,
        )
        .await
        {
            last_success = Some(Instant::now());
        }
    }
}

fn push_span(batch: &mut Vec<SpanData>, queued: QueuedSpan, scope: &InstrumentationScope) {
    if let Some(span) = span_data_from_snapshot(
        &queued.snapshot,
        queued.baggage,
        queued.start_time,
        queued.end_time,
        scope.clone(),
    ) {
        batch.push(span);
    }
}

/// 在限定重试窗口内导出整批 Span，并同步 exporter 健康状态。
async fn export_batch(
    exporter: &opentelemetry_otlp::SpanExporter,
    batch: &mut Vec<SpanData>,
    metrics: &dyn AsyncMetricSink,
    export_success_logged: &mut bool,
) -> bool {
    if batch.is_empty() {
        return true;
    }
    let spans = std::mem::take(batch);
    record_histogram(metrics, MetricId::TracesExportBatchSize, spans.len() as f64);
    let started = Instant::now();
    let mut last_error = None;
    for attempt in 1..=8 {
        match exporter.export(spans.clone()).await {
            Ok(()) => {
                if !*export_success_logged {
                    tracing::info!(
                        marker = "HELIX_OTEL_EXPORT_OK",
                        span_count = spans.len(),
                        "HELIX_OTEL_EXPORT_OK"
                    );
                    *export_success_logged = true;
                }
                record_histogram(
                    metrics,
                    MetricId::TracesExportDurationSeconds,
                    started.elapsed().as_secs_f64(),
                );
                record_gauge(metrics, MetricId::TracesLastSuccessAgeSeconds, 0.0);
                record_gauge(metrics, MetricId::TracesExporterState, 1.0);
                return true;
            }
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
            }
        }
    }
    if let Some(error) = last_error {
        record_histogram(
            metrics,
            MetricId::TracesExportDurationSeconds,
            started.elapsed().as_secs_f64(),
        );
        record_counter(metrics, MetricId::TracesExportErrorsTotal, 1.0);
        record_gauge(metrics, MetricId::TracesExporterState, 2.0);
        tracing::warn!(
            error = %error,
            span_count = spans.len(),
            "helix otel span batch export failed after retries"
        );
    }
    false
}

/// 记录 trace exporter Counter，保持固定 exporter 标签。
fn record_counter(metrics: &dyn AsyncMetricSink, id: MetricId, value: f64) {
    if metrics.is_enabled() {
        let _ = metrics.try_record(MetricEvent::counter(id, value, exporter_labels()));
    }
}

/// 记录 trace exporter Gauge，热路径只执行 try_record。
fn record_gauge(metrics: &dyn AsyncMetricSink, id: MetricId, value: f64) {
    if metrics.is_enabled() {
        let _ = metrics.try_record(MetricEvent::gauge(id, value, exporter_labels()));
    }
}

/// 记录 trace exporter Histogram，不等待 metrics exporter。
fn record_histogram(metrics: &dyn AsyncMetricSink, id: MetricId, value: f64) {
    if metrics.is_enabled() {
        let _ = metrics.try_record(MetricEvent::histogram(id, value, exporter_labels()));
    }
}

/// 返回冻结的 trace exporter 标签集合。
fn exporter_labels() -> MetricLabels {
    MetricLabels::one(LabelKey::Stage, "telemetry").with(LabelKey::Operation, "trace_export")
}

fn span_data_from_snapshot(
    snapshot: &HostSpanSnapshot,
    baggage: Option<String>,
    start_time: SystemTime,
    end_time: SystemTime,
    scope: InstrumentationScope,
) -> Option<SpanData> {
    let trace_id = TraceId::from_hex(snapshot.trace_id.as_deref()?).ok()?;
    let span_id = SpanId::from_hex(&snapshot.span_id).ok()?;
    let parent_span_id = snapshot
        .parent_span_id
        .as_deref()
        .and_then(|id| SpanId::from_hex(id).ok())
        .unwrap_or(SpanId::INVALID);
    let mut attributes = vec![
        KeyValue::new("service.layer", "helix"),
        KeyValue::new("span.direction", format!("{:?}", snapshot.direction)),
    ];
    if let Some(baggage) = baggage {
        attributes.push(KeyValue::new("baggage", baggage));
    }
    attributes.extend(
        snapshot
            .attributes
            .iter()
            .map(|(key, value)| KeyValue::new(key.clone(), value.clone())),
    );

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
        name: snapshot.name.clone().into(),
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
