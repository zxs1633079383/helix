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
                        export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut batch).await;
                    }
                }
                _ = interval.tick() => {
                    export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut batch).await;
                }
            }
        }
        while let Ok(event) = rx.try_recv() {
            stats.depth.fetch_sub(1, Ordering::Relaxed);
            batch.push(event);
            if batch.len() >= config.batch_max.max(1) {
                export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut batch).await;
            }
        }
        export_batch(&config, &mut exporter, &stats, &mut self_cursor, &mut batch).await;
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
    batch: &mut Vec<MetricEvent>,
) {
    if batch.is_empty() {
        return;
    }
    append_self_metrics(batch, stats, self_cursor);
    let event_count = batch.len() as u64;
    let start = std::time::Instant::now();
    let mut result = exporter.export_batch(batch);
    for attempt in 0..config.retry_max {
        if result.is_ok() {
            break;
        }
        stats
            .state
            .store(MetricExporterState::Backoff as u64, Ordering::Relaxed);
        let shift = attempt.min(16);
        let backoff =
            Duration::from_millis(100_u64.saturating_mul(1_u64 << shift)).min(config.max_backoff);
        tokio::time::sleep(backoff).await;
        result = exporter.export_batch(batch);
    }
    stats.duration_ns.store(
        start.elapsed().as_nanos().min(u64::MAX as u128) as u64,
        Ordering::Relaxed,
    );
    if result.is_ok() {
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
    }
    batch.clear();
}

#[derive(Default)]
struct SelfMetricCursor {
    dropped: u64,
    errors: u64,
}

fn append_self_metrics(
    batch: &mut Vec<MetricEvent>,
    stats: &RuntimeStats,
    cursor: &mut SelfMetricCursor,
) {
    let snapshot = stats.snapshot();
    let labels = MetricLabels::one(LabelKey::Stage, "metrics_export");
    let batch_size = batch.len() as f64;
    let dropped_delta = snapshot.dropped_total.saturating_sub(cursor.dropped);
    let error_delta = snapshot.export_errors_total.saturating_sub(cursor.errors);
    cursor.dropped = snapshot.dropped_total;
    cursor.errors = snapshot.export_errors_total;
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
            dropped_delta as f64,
            labels.with(LabelKey::ErrorKind, "queue_full"),
        ),
        MetricEvent::counter(
            MetricId::MetricsExportErrorsTotal,
            error_delta as f64,
            labels.with(LabelKey::ErrorKind, "export_failed"),
        ),
    ]);
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
