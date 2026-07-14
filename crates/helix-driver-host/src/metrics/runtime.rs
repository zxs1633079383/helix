use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc as std_mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

use super::worker::run_worker;
use super::{AsyncMetricSink, MetricEvent, NoopMetricSink, OtlpMetricExporter, RecordOutcome};

#[derive(Clone, Debug)]
pub struct MetricRuntimeConfig {
    pub enabled: bool,
    pub service_name: String,
    pub endpoint: String,
    pub deployment_environment: String,
    pub platform: String,
    pub scenario: String,
    pub profile: String,
    pub queue_capacity: usize,
    pub batch_max: usize,
    pub export_interval: Duration,
    pub export_timeout: Duration,
    pub retry_max: u32,
    pub max_backoff: Duration,
    pub circuit_failure_threshold: u32,
    pub circuit_open_duration: Duration,
}

impl Default for MetricRuntimeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: "helix-driver-host".to_string(),
            endpoint: "http://127.0.0.1:4317".to_string(),
            deployment_environment: "local".to_string(),
            platform: "host".to_string(),
            scenario: "canary".to_string(),
            profile: "baseline".to_string(),
            queue_capacity: 8192,
            batch_max: 512,
            export_interval: Duration::from_secs(5),
            export_timeout: Duration::from_secs(2),
            retry_max: 3,
            max_backoff: Duration::from_secs(30),
            circuit_failure_threshold: 3,
            circuit_open_duration: Duration::from_secs(15),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum MetricExporterState {
    Disabled = 0,
    Running = 1,
    Backoff = 2,
    Stopped = 3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricRuntimeSnapshot {
    pub queue_depth: usize,
    pub queue_capacity: usize,
    pub dropped_total: u64,
    pub exported_batches_total: u64,
    pub exported_events_total: u64,
    pub export_errors_total: u64,
    pub last_export_duration_ns: u64,
    pub last_success_unix_ns: u64,
    pub exporter_state: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownOutcome {
    Disabled,
    Drained,
    TimedOut,
}

#[derive(Debug, Error)]
#[error("metrics export failed: {message}")]
pub struct MetricExportError {
    pub message: String,
}

#[derive(Debug, Error)]
pub enum MetricRuntimeError {
    #[error("metrics worker thread init failed: {0}")]
    ThreadInit(#[from] std::io::Error),
}

pub trait BatchMetricExporter: Send + 'static {
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError>;

    fn retry_flush(&mut self) -> Result<(), MetricExportError> {
        Err(MetricExportError {
            message: "exporter does not support record-once flush retry".to_string(),
        })
    }

    fn shutdown(&mut self, _timeout: Duration) -> Result<(), MetricExportError> {
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct RuntimeStats {
    pub(super) depth: AtomicUsize,
    pub(super) capacity: AtomicUsize,
    pub(super) dropped: AtomicU64,
    pub(super) circuit_dropped: AtomicU64,
    pub(super) batches: AtomicU64,
    pub(super) events: AtomicU64,
    pub(super) errors: AtomicU64,
    pub(super) duration_ns: AtomicU64,
    pub(super) last_success_ns: AtomicU64,
    pub(super) state: AtomicU64,
}

impl RuntimeStats {
    pub(super) fn snapshot(&self) -> MetricRuntimeSnapshot {
        MetricRuntimeSnapshot {
            queue_depth: self.depth.load(Ordering::Relaxed),
            queue_capacity: self.capacity.load(Ordering::Relaxed),
            dropped_total: self.dropped.load(Ordering::Relaxed),
            exported_batches_total: self.batches.load(Ordering::Relaxed),
            exported_events_total: self.events.load(Ordering::Relaxed),
            export_errors_total: self.errors.load(Ordering::Relaxed),
            last_export_duration_ns: self.duration_ns.load(Ordering::Relaxed),
            last_success_unix_ns: self.last_success_ns.load(Ordering::Relaxed),
            exporter_state: self.state.load(Ordering::Relaxed),
        }
    }
}

struct BoundedMetricSink {
    tx: mpsc::Sender<MetricEvent>,
    stats: Arc<RuntimeStats>,
}

impl AsyncMetricSink for BoundedMetricSink {
    #[inline]
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.stats.depth.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(event) {
            Ok(()) => RecordOutcome::Accepted,
            Err(_) => {
                self.stats.depth.fetch_sub(1, Ordering::Relaxed);
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                RecordOutcome::DroppedQueueFull
            }
        }
    }
}

pub struct BoundedMetricRuntime {
    sink: Arc<dyn AsyncMetricSink>,
    stats: Arc<RuntimeStats>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    completion_rx: Option<std_mpsc::Receiver<()>>,
    worker: Option<JoinHandle<()>>,
    enabled: bool,
}

impl BoundedMetricRuntime {
    pub fn start_otlp(config: MetricRuntimeConfig) -> Result<Self, MetricRuntimeError> {
        let exporter = OtlpMetricExporter::new(config.clone());
        Self::start_with_exporter(config, exporter)
    }

    pub fn start_with_exporter<E>(
        config: MetricRuntimeConfig,
        exporter: E,
    ) -> Result<Self, MetricRuntimeError>
    where
        E: BatchMetricExporter,
    {
        if !config.enabled {
            return Ok(Self::disabled());
        }
        let capacity = config.queue_capacity.max(1);
        let (tx, rx) = mpsc::channel(capacity);
        let stats = Arc::new(RuntimeStats::default());
        stats.capacity.store(capacity, Ordering::Relaxed);
        stats
            .state
            .store(MetricExporterState::Running as u64, Ordering::Relaxed);
        let sink: Arc<dyn AsyncMetricSink> = Arc::new(BoundedMetricSink {
            tx,
            stats: Arc::clone(&stats),
        });
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let (completion_tx, completion_rx) = std_mpsc::channel();
        let worker_stats = Arc::clone(&stats);
        let worker = std::thread::Builder::new()
            .name("helix-metrics-worker".to_string())
            .spawn(move || {
                run_worker(config, rx, shutdown_rx, exporter, worker_stats);
                completion_tx.send(()).ok();
            })?;
        Ok(Self {
            sink,
            stats,
            shutdown_tx: Some(shutdown_tx),
            completion_rx: Some(completion_rx),
            worker: Some(worker),
            enabled: true,
        })
    }

    pub fn disabled() -> Self {
        let stats = Arc::new(RuntimeStats::default());
        stats
            .state
            .store(MetricExporterState::Disabled as u64, Ordering::Relaxed);
        Self {
            sink: Arc::new(NoopMetricSink),
            stats,
            shutdown_tx: None,
            completion_rx: None,
            worker: None,
            enabled: false,
        }
    }

    pub fn sink(&self) -> Arc<dyn AsyncMetricSink> {
        Arc::clone(&self.sink)
    }

    pub fn snapshot(&self) -> MetricRuntimeSnapshot {
        self.stats.snapshot()
    }

    pub fn shutdown(mut self, timeout: Duration) -> ShutdownOutcome {
        if !self.enabled {
            return ShutdownOutcome::Disabled;
        }
        self.shutdown_tx.take().and_then(|tx| tx.send(()).ok());
        let completed = self
            .completion_rx
            .take()
            .is_some_and(|rx| rx.recv_timeout(timeout).is_ok());
        if completed {
            if let Some(worker) = self.worker.take() {
                worker.join().ok();
            }
            ShutdownOutcome::Drained
        } else {
            self.worker.take();
            ShutdownOutcome::TimedOut
        }
    }
}

impl Drop for BoundedMetricRuntime {
    fn drop(&mut self) {
        self.shutdown_tx.take().and_then(|tx| tx.send(()).ok());
        self.worker.take();
    }
}
