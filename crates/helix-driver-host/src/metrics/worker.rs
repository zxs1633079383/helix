use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::{mpsc, oneshot};

use super::runtime::{BatchMetricExporter, MetricExporterState, MetricRuntimeConfig, RuntimeStats};
use super::{LabelKey, MetricEvent, MetricId, MetricLabels};

pub(super) fn run_worker<E: BatchMetricExporter>(
    config: MetricRuntimeConfig,
    mut rx: mpsc::Receiver<MetricEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
    mut exporter: E,
    stats: Arc<RuntimeStats>,
) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("helix-metrics-otlp")
        .build();
    let Ok(runtime) = runtime else {
        stats
            .state
            .store(MetricExporterState::Stopped as u64, Ordering::Relaxed);
        return;
    };
    runtime.block_on(async {
        let mut batch = Vec::with_capacity(config.batch_max.max(1) + 8);
        let mut self_cursor = SelfMetricCursor::default();
        let mut circuit = ExportCircuit::default();
        let mut interval = tokio::time::interval(config.export_interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                _ = &mut shutdown_rx => break,
                event = rx.recv() => {
                    let Some(event) = event else { break; };
                    stats.depth.fetch_sub(1, Ordering::Relaxed);
                    batch.push(event);
                    drain_ready(&config, &mut rx, &stats, &mut batch);
                    if batch.len() >= config.batch_max.max(1) {
                        export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut circuit, &mut batch).await;
                    }
                }
                _ = interval.tick() => {
                    export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut circuit, &mut batch).await;
                }
            }
        }
        rx.close();
        while let Ok(event) = rx.try_recv() {
            stats.depth.fetch_sub(1, Ordering::Relaxed);
            batch.push(event);
            if batch.len() >= config.batch_max.max(1) {
                export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut circuit, &mut batch).await;
            }
        }
        export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut circuit, &mut batch).await;
        exporter.shutdown(config.export_timeout).ok();
        stats
            .state
            .store(MetricExporterState::Stopped as u64, Ordering::Relaxed);
    });
}

fn drain_ready(
    config: &MetricRuntimeConfig,
    rx: &mut mpsc::Receiver<MetricEvent>,
    stats: &RuntimeStats,
    batch: &mut Vec<MetricEvent>,
) {
    while batch.len() < config.batch_max.max(1) {
        match rx.try_recv() {
            Ok(event) => {
                stats.depth.fetch_sub(1, Ordering::Relaxed);
                batch.push(event);
            }
            Err(_) => break,
        }
    }
}

async fn export_batch<E: BatchMetricExporter>(
    config: &MetricRuntimeConfig,
    exporter: &mut E,
    stats: &RuntimeStats,
    self_cursor: &mut SelfMetricCursor,
    circuit: &mut ExportCircuit,
    batch: &mut Vec<MetricEvent>,
) {
    if circuit.rejects_now() {
        let dropped = batch.len() as u64;
        stats.dropped.fetch_add(dropped, Ordering::Relaxed);
        stats.circuit_dropped.fetch_add(dropped, Ordering::Relaxed);
        stats
            .state
            .store(MetricExporterState::Open as u64, Ordering::Relaxed);
        batch.clear();
        return;
    }
    let next_cursor = append_self_metrics(batch, stats, self_cursor);
    let event_count = batch.len() as u64;
    let start = std::time::Instant::now();
    let mut result = exporter.export_batch(batch);
    for attempt in 0..config.retry_max {
        if result.is_ok() {
            break;
        }
        if start.elapsed() >= config.export_timeout {
            break;
        }
        stats
            .state
            .store(MetricExporterState::Backoff as u64, Ordering::Relaxed);
        let shift = attempt.min(16);
        let jitter_ms = u64::from(attempt.wrapping_mul(17) % 41);
        let backoff = Duration::from_millis(
            100_u64
                .saturating_mul(1_u64 << shift)
                .saturating_add(jitter_ms),
        )
        .min(config.max_backoff);
        let remaining = config.export_timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            break;
        }
        tokio::time::sleep(backoff.min(remaining)).await;
        if start.elapsed() >= config.export_timeout {
            break;
        }
        result = exporter.retry_flush();
    }
    stats.duration_ns.store(
        start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
    if result.is_ok() {
        *self_cursor = next_cursor;
        if !self_cursor.export_success_logged {
            tracing::info!(
                marker = "HELIX_METRICS_EXPORT_OK",
                event_count,
                "HELIX_METRICS_EXPORT_OK"
            );
            self_cursor.export_success_logged = true;
        }
        circuit.on_success();
        stats.batches.fetch_add(1, Ordering::Relaxed);
        stats.events.fetch_add(event_count, Ordering::Relaxed);
        stats
            .last_success_ns
            .store(unix_time_ns(), Ordering::Relaxed);
        stats
            .state
            .store(MetricExporterState::Running as u64, Ordering::Relaxed);
    } else {
        stats.errors.fetch_add(1, Ordering::Relaxed);
        let state = if circuit.on_failure(config) {
            MetricExporterState::Open
        } else {
            MetricExporterState::Backoff
        };
        stats.state.store(state as u64, Ordering::Relaxed);
    }
    batch.clear();
}

#[derive(Default)]
struct SelfMetricCursor {
    dropped: u64,
    circuit_dropped: u64,
    errors: u64,
    export_success_logged: bool,
}

#[derive(Default)]
struct ExportCircuit {
    consecutive_failures: u32,
    open_until: Option<std::time::Instant>,
}

impl ExportCircuit {
    fn rejects_now(&mut self) -> bool {
        match self.open_until {
            Some(until) if std::time::Instant::now() < until => true,
            Some(_) => {
                self.open_until = None;
                false
            }
            None => false,
        }
    }

    fn on_success(&mut self) {
        self.consecutive_failures = 0;
        self.open_until = None;
    }

    fn on_failure(&mut self, config: &MetricRuntimeConfig) -> bool {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= config.circuit_failure_threshold.max(1) {
            self.open_until = Some(std::time::Instant::now() + config.circuit_open_duration);
            self.consecutive_failures = 0;
            true
        } else {
            false
        }
    }
}

fn append_self_metrics(
    batch: &mut Vec<MetricEvent>,
    stats: &RuntimeStats,
    cursor: &SelfMetricCursor,
) -> SelfMetricCursor {
    let snapshot = stats.snapshot();
    let labels = MetricLabels::one(LabelKey::Stage, "telemetry");
    let batch_size = batch.len() as f64;
    let dropped_delta = snapshot.dropped_total.saturating_sub(cursor.dropped);
    let circuit_dropped = stats.circuit_dropped.load(Ordering::Relaxed);
    let circuit_dropped_delta = circuit_dropped.saturating_sub(cursor.circuit_dropped);
    let queue_dropped_delta = dropped_delta.saturating_sub(circuit_dropped_delta);
    let error_delta = snapshot.export_errors_total.saturating_sub(cursor.errors);
    batch.extend([
        MetricEvent::gauge(
            MetricId::MetricsQueueDepth,
            snapshot.queue_depth as f64,
            labels,
        ),
        MetricEvent::gauge(
            MetricId::MetricsQueueCapacity,
            snapshot.queue_capacity as f64,
            labels,
        ),
        MetricEvent::histogram(MetricId::MetricsExportBatchSize, batch_size, labels),
        MetricEvent::histogram(
            MetricId::MetricsExportDurationSeconds,
            snapshot.last_export_duration_ns as f64 / 1_000_000_000.0,
            labels,
        ),
        MetricEvent::gauge(
            MetricId::MetricsExporterState,
            snapshot.exporter_state as f64,
            labels,
        ),
        MetricEvent::gauge(
            MetricId::MetricsLastSuccessAgeSeconds,
            last_success_age_seconds(snapshot.last_success_unix_ns),
            labels,
        ),
        MetricEvent::counter(
            MetricId::MetricsDroppedTotal,
            queue_dropped_delta as f64,
            labels.with(LabelKey::ErrorKind, "queue_full"),
        ),
        MetricEvent::counter(
            MetricId::MetricsDroppedTotal,
            circuit_dropped_delta as f64,
            labels.with(LabelKey::ErrorKind, "circuit_open"),
        ),
        MetricEvent::counter(
            MetricId::MetricsExportErrorsTotal,
            error_delta as f64,
            labels.with(LabelKey::ErrorKind, "export_failed"),
        ),
        MetricEvent::counter(MetricId::TelemetryCanaryTotal, 1.0, labels),
    ]);
    SelfMetricCursor {
        dropped: snapshot.dropped_total,
        circuit_dropped,
        errors: snapshot.export_errors_total,
        export_success_logged: cursor.export_success_logged,
    }
}

fn unix_time_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

fn last_success_age_seconds(last_success_ns: u64) -> f64 {
    if last_success_ns == 0 {
        return 0.0;
    }
    unix_time_ns().saturating_sub(last_success_ns) as f64 / 1_000_000_000.0
}
