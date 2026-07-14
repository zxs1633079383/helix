use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use helix_driver_host::metrics::{
    BatchMetricExporter, BoundedMetricRuntime, LabelKey, MetricEvent, MetricExportError, MetricId,
    MetricLabels, MetricRuntimeConfig, RecordOutcome, ShutdownOutcome,
};

#[derive(Clone, Default)]
struct RecordingExporter {
    batches: Arc<Mutex<Vec<Vec<MetricEvent>>>>,
}

impl BatchMetricExporter for RecordingExporter {
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        self.batches.lock().unwrap().push(batch.to_vec());
        Ok(())
    }
}

struct PausedExporter {
    entered: mpsc::Sender<()>,
    resume: Option<mpsc::Receiver<()>>,
}

impl BatchMetricExporter for PausedExporter {
    fn export_batch(&mut self, _batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        if let Some(resume) = self.resume.take() {
            self.entered.send(()).ok();
            resume.recv().ok();
        }
        Ok(())
    }
}

fn enabled_config() -> MetricRuntimeConfig {
    MetricRuntimeConfig {
        enabled: true,
        queue_capacity: 8,
        batch_max: 2,
        export_interval: Duration::from_secs(60),
        retry_max: 0,
        ..MetricRuntimeConfig::default()
    }
}

fn operation() -> MetricEvent {
    MetricEvent::counter(
        MetricId::OperationsTotal,
        1.0,
        MetricLabels::one(LabelKey::Layer, "L2")
            .with(LabelKey::Stage, "engine")
            .with(LabelKey::Operation, "step"),
    )
}

#[test]
fn disabled_runtime_is_noop_without_worker() {
    let runtime = BoundedMetricRuntime::start_with_exporter(
        MetricRuntimeConfig::default(),
        RecordingExporter::default(),
    )
    .unwrap();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Disabled
    );
    assert_eq!(runtime.snapshot().queue_capacity, 0);
    assert_eq!(
        runtime.shutdown(Duration::from_millis(10)),
        ShutdownOutcome::Disabled
    );
}

#[test]
fn worker_exports_batches_and_self_observation() {
    let exporter = RecordingExporter::default();
    let batches = Arc::clone(&exporter.batches);
    let runtime = BoundedMetricRuntime::start_with_exporter(enabled_config(), exporter).unwrap();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    for _ in 0..50 {
        if runtime.snapshot().exported_batches_total > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        runtime.shutdown(Duration::from_secs(1)),
        ShutdownOutcome::Drained
    );
    let flattened: Vec<_> = batches.lock().unwrap().iter().flatten().copied().collect();
    assert!(flattened
        .iter()
        .any(|event| event.id == MetricId::OperationsTotal));
    assert!(flattened
        .iter()
        .any(|event| event.id == MetricId::MetricsExportBatchSize));
    assert!(flattened
        .iter()
        .any(|event| event.id == MetricId::MetricsQueueCapacity));
}

#[test]
fn paused_worker_drops_metrics_without_blocking_caller() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let config = MetricRuntimeConfig {
        queue_capacity: 1,
        batch_max: 1,
        ..enabled_config()
    };
    let runtime = BoundedMetricRuntime::start_with_exporter(
        config,
        PausedExporter {
            entered: entered_tx,
            resume: Some(resume_rx),
        },
    )
    .unwrap();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    let started = std::time::Instant::now();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::DroppedQueueFull
    );
    assert!(started.elapsed() < Duration::from_millis(10));
    assert_eq!(runtime.snapshot().dropped_total, 1);
    resume_tx.send(()).unwrap();
    assert_eq!(
        runtime.shutdown(Duration::from_secs(1)),
        ShutdownOutcome::Drained
    );
}
