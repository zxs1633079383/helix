use std::sync::atomic::{AtomicUsize, Ordering};
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

#[derive(Clone, Default)]
struct RetryState {
    recorded_operation_batches: Arc<Mutex<usize>>,
    flush_attempts: Arc<Mutex<usize>>,
}

struct FlushFailsOnceExporter {
    state: RetryState,
}

#[derive(Clone, Default)]
struct SequenceState {
    calls: Arc<AtomicUsize>,
    successful_batches: Arc<Mutex<Vec<Vec<MetricEvent>>>>,
}

struct FailThenSucceedExporter(SequenceState);

impl BatchMetricExporter for FailThenSucceedExporter {
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        let call = self.0.calls.fetch_add(1, Ordering::Relaxed);
        if call == 0 {
            return Err(MetricExportError {
                message: "intentional first failure".to_string(),
            });
        }
        self.0
            .successful_batches
            .lock()
            .unwrap()
            .push(batch.to_vec());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct AlwaysFailState(Arc<AtomicUsize>);

struct AlwaysFailExporter(AlwaysFailState);

impl BatchMetricExporter for AlwaysFailExporter {
    fn export_batch(&mut self, _batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        self.0 .0.fetch_add(1, Ordering::Relaxed);
        Err(MetricExportError {
            message: "collector unavailable".to_string(),
        })
    }
}

impl BatchMetricExporter for FlushFailsOnceExporter {
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        if batch
            .iter()
            .any(|event| event.id == MetricId::OperationsTotal)
        {
            *self.state.recorded_operation_batches.lock().unwrap() += 1;
        }
        Err(MetricExportError {
            message: "first flush failed".to_string(),
        })
    }

    fn retry_flush(&mut self) -> Result<(), MetricExportError> {
        *self.state.flush_attempts.lock().unwrap() += 1;
        Ok(())
    }
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
    assert!(flattened
        .iter()
        .any(|event| event.id == MetricId::TelemetryCanaryTotal));
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

#[test]
fn retry_flush_does_not_record_batch_twice() {
    let state = RetryState::default();
    let mut config = enabled_config();
    config.batch_max = 1;
    config.retry_max = 1;
    config.export_timeout = Duration::from_secs(1);
    let runtime = BoundedMetricRuntime::start_with_exporter(
        config,
        FlushFailsOnceExporter {
            state: state.clone(),
        },
    )
    .unwrap();
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
    assert_eq!(*state.recorded_operation_batches.lock().unwrap(), 1);
    assert!(*state.flush_attempts.lock().unwrap() >= 1);
}

#[test]
fn self_metric_cursor_advances_only_after_success() {
    let state = SequenceState::default();
    let mut config = enabled_config();
    config.batch_max = 1;
    config.circuit_failure_threshold = 3;
    let runtime =
        BoundedMetricRuntime::start_with_exporter(config, FailThenSucceedExporter(state.clone()))
            .unwrap();
    for _ in 0..50 {
        if runtime.snapshot().export_errors_total > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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
    let batches = state.successful_batches.lock().unwrap();
    assert!(batches
        .iter()
        .flatten()
        .any(|event| { event.id == MetricId::MetricsExportErrorsTotal && event.value >= 1.0 }));
}

#[test]
fn circuit_opens_and_drops_without_blocking_producer() {
    let state = AlwaysFailState::default();
    let mut config = enabled_config();
    config.batch_max = 1;
    config.circuit_failure_threshold = 1;
    config.circuit_open_duration = Duration::from_secs(5);
    let runtime =
        BoundedMetricRuntime::start_with_exporter(config, AlwaysFailExporter(state.clone()))
            .unwrap();
    for _ in 0..50 {
        if runtime.snapshot().export_errors_total > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let calls_before = state.0.load(Ordering::Relaxed);
    let started = std::time::Instant::now();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    for _ in 0..50 {
        if runtime.snapshot().dropped_total > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.elapsed() < Duration::from_millis(600));
    assert_eq!(state.0.load(Ordering::Relaxed), calls_before);
    assert!(runtime.snapshot().dropped_total >= 1);
    assert_eq!(
        runtime.shutdown(Duration::from_secs(1)),
        ShutdownOutcome::Drained
    );
}

struct RetainingFailTwiceExporter {
    calls: usize,
    recorded: Arc<Mutex<Vec<Vec<MetricEvent>>>>,
}

impl BatchMetricExporter for RetainingFailTwiceExporter {
    // 模拟 SDK 已接受记录但前两次网络导出失败，所有 batch 都保留在累计聚合器。
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        self.recorded.lock().unwrap().push(batch.to_vec());
        self.calls += 1;
        if self.calls <= 2 {
            Err(MetricExportError {
                message: "recorded then network failed".to_string(),
            })
        } else {
            Ok(())
        }
    }

    // 失败只涉及网络，recorded 中的自观测增量不能在后续批次重复计入。
    fn retains_failed_batch(&self) -> bool {
        true
    }
}

// 两次连续失败后恢复，累计错误数必须等于实际失败数，业务事件只记录一次。
#[test]
fn retained_failed_batches_do_not_repeat_self_metric_deltas() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let config = MetricRuntimeConfig {
        export_interval: Duration::from_millis(10),
        circuit_failure_threshold: 10,
        ..enabled_config()
    };
    let runtime = BoundedMetricRuntime::start_with_exporter(
        config,
        RetainingFailTwiceExporter {
            calls: 0,
            recorded: Arc::clone(&recorded),
        },
    )
    .unwrap();
    assert_eq!(
        runtime.sink().try_record(operation()),
        RecordOutcome::Accepted
    );
    for _ in 0..100 {
        if runtime.snapshot().exported_batches_total > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(runtime.snapshot().export_errors_total, 2);
    assert!(runtime.snapshot().exported_batches_total > 0);
    assert_eq!(
        runtime.shutdown(Duration::from_secs(1)),
        ShutdownOutcome::Drained
    );
    let batches = recorded.lock().unwrap();
    let errors: f64 = batches
        .iter()
        .flatten()
        .filter(|e| e.id == MetricId::MetricsExportErrorsTotal)
        .map(|e| e.value)
        .sum();
    let operations: f64 = batches
        .iter()
        .flatten()
        .filter(|e| e.id == MetricId::OperationsTotal)
        .map(|e| e.value)
        .sum();
    assert_eq!(errors, 2.0);
    assert_eq!(operations, 1.0);
}
