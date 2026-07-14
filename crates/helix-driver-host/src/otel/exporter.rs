use std::sync::atomic::{AtomicU64, Ordering};
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

#[derive(Debug)]
pub(super) struct OtlpExporter {
    sender: Option<mpsc::Sender<QueuedSpan>>,
    dropped_spans: AtomicU64,
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
}

impl OtlpExporter {
    pub(super) fn new(config: &HostOtelConfig) -> Option<Self> {
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
        runtime.spawn(run_export_worker(receiver, exporter, scope));

        Some(Self {
            sender: Some(sender),
            dropped_spans: AtomicU64::new(0),
            runtime: Mutex::new(Some(runtime)),
        })
    }

    pub(super) fn try_enqueue(
        &self,
        snapshot: HostSpanSnapshot,
        baggage: Option<String>,
        start_time: SystemTime,
        end_time: SystemTime,
    ) {
        let queued = QueuedSpan {
            snapshot,
            baggage,
            start_time,
            end_time,
        };
        let accepted = self
            .sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(queued).is_ok());
        if !accepted {
            self.dropped_spans.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(super) fn dropped_span_count(&self) -> u64 {
        self.dropped_spans.load(Ordering::Relaxed)
    }
}

impl Drop for OtlpExporter {
    fn drop(&mut self) {
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
) {
    let mut batch = Vec::with_capacity(EXPORT_BATCH_SIZE);
    while let Some(first) = receiver.recv().await {
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
                }
                _ = &mut flush_deadline => break,
            }
        }

        export_batch(&exporter, &mut batch).await;
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

async fn export_batch(exporter: &opentelemetry_otlp::SpanExporter, batch: &mut Vec<SpanData>) {
    if batch.is_empty() {
        return;
    }
    let spans = std::mem::take(batch);
    let mut last_error = None;
    for attempt in 1..=8 {
        match exporter.export(spans.clone()).await {
            Ok(()) => return,
            Err(error) => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(250 * attempt)).await;
            }
        }
    }
    if let Some(error) = last_error {
        tracing::warn!(
            error = %error,
            span_count = spans.len(),
            "helix otel span batch export failed after retries"
        );
    }
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
