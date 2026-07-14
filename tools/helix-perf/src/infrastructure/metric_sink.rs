#[cfg(not(feature = "sdk-metrics"))]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(not(feature = "sdk-metrics"))]
use std::sync::mpsc;
use std::sync::Arc;
#[cfg(not(feature = "sdk-metrics"))]
use std::thread;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub enum TelemetryMode {
    Disabled,
    Enabled { capacity: usize, pause_worker: bool },
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProbeSnapshot {
    pub accepted: u64,
    pub dropped: u64,
    pub exported: u64,
    pub batches: u64,
    pub depth: usize,
    pub capacity: usize,
}

#[cfg(not(feature = "sdk-metrics"))]
pub struct MetricProbe {
    sender: Option<mpsc::SyncSender<u64>>,
    accepted: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
    exported: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    capacity: usize,
    paused_receiver: Option<mpsc::Receiver<u64>>,
}

#[cfg(not(feature = "sdk-metrics"))]
impl MetricProbe {
    pub fn new(mode: TelemetryMode) -> Self {
        let TelemetryMode::Enabled {
            capacity,
            pause_worker,
        } = mode
        else {
            return Self::disabled();
        };
        let (sender, receiver) = mpsc::sync_channel::<u64>(capacity);
        let accepted = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let exported = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let paused_receiver = if pause_worker {
            Some(receiver)
        } else {
            let exported_worker = Arc::clone(&exported);
            let stop_worker = Arc::clone(&stop);
            thread::Builder::new()
                .name("helix-perf-metric-probe".into())
                .spawn(move || {
                    while !stop_worker.load(Ordering::Relaxed) {
                        match receiver.recv_timeout(Duration::from_millis(10)) {
                            Ok(_) => {
                                exported_worker.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    while receiver.try_recv().is_ok() {
                        exported_worker.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .expect("metric probe worker must start");
            None
        };
        Self {
            sender: Some(sender),
            accepted,
            dropped,
            exported,
            stop,
            capacity,
            paused_receiver,
        }
    }

    fn disabled() -> Self {
        Self {
            sender: None,
            accepted: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
            exported: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            capacity: 0,
            paused_receiver: None,
        }
    }

    #[inline]
    pub fn try_record(&self, value: u64) {
        let Some(sender) = &self.sender else { return };
        match sender.try_send(value) {
            Ok(()) => {
                self.accepted.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub fn snapshot(&self) -> ProbeSnapshot {
        let _receiver_is_held = self.paused_receiver.is_some();
        ProbeSnapshot {
            accepted: self.accepted.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            exported: self.exported.load(Ordering::Relaxed),
            batches: self.exported.load(Ordering::Relaxed),
            depth: self
                .accepted
                .load(Ordering::Relaxed)
                .saturating_sub(self.exported.load(Ordering::Relaxed)) as usize,
            capacity: self.capacity,
        }
    }
}

#[cfg(not(feature = "sdk-metrics"))]
impl Drop for MetricProbe {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(feature = "sdk-metrics")]
use helix_driver_host::metrics::{
    AsyncMetricSink, BatchMetricExporter, BoundedMetricRuntime, LabelKey, MetricEvent,
    MetricExportError, MetricId, MetricLabels, MetricRuntimeConfig,
};

#[cfg(feature = "sdk-metrics")]
pub struct MetricProbe {
    runtime: Option<BoundedMetricRuntime>,
    sink: Arc<dyn AsyncMetricSink>,
}

#[cfg(feature = "sdk-metrics")]
impl MetricProbe {
    pub fn new(mode: TelemetryMode) -> Self {
        let (enabled, capacity, pause_worker) = match mode {
            TelemetryMode::Disabled => (false, 1, false),
            TelemetryMode::Enabled {
                capacity,
                pause_worker,
            } => (true, capacity, pause_worker),
        };
        let config = MetricRuntimeConfig {
            enabled,
            service_name: "helix-perf".to_string(),
            deployment_environment: "pre".to_string(),
            platform: "native".to_string(),
            scenario: "l2_mixed".to_string(),
            channel_shape: "parallel-50".to_string(),
            profile: "steady".to_string(),
            queue_capacity: capacity,
            batch_max: 512,
            export_interval: Duration::from_millis(100),
            export_timeout: Duration::from_millis(100),
            retry_max: 0,
            max_backoff: Duration::from_millis(10),
            ..MetricRuntimeConfig::default()
        };
        let runtime = if pause_worker {
            BoundedMetricRuntime::start_with_exporter(config, ProbeExporter)
        } else {
            BoundedMetricRuntime::start_otlp(config)
        }
        .expect("metric SDK worker must start");
        let sink = runtime.sink();
        Self {
            runtime: Some(runtime),
            sink,
        }
    }

    pub fn snapshot(&self) -> ProbeSnapshot {
        let snapshot = self.runtime.as_ref().expect("runtime present").snapshot();
        ProbeSnapshot {
            accepted: snapshot.exported_events_total
                + snapshot.queue_depth as u64
                + snapshot.dropped_total,
            dropped: snapshot.dropped_total,
            exported: snapshot.exported_events_total,
            batches: snapshot.exported_batches_total,
            depth: snapshot.queue_depth,
            capacity: snapshot.queue_capacity,
        }
    }

    pub fn sink(&self) -> Arc<dyn AsyncMetricSink> {
        Arc::clone(&self.sink)
    }

    pub fn record_final(
        &self,
        correctness: &std::collections::BTreeMap<String, u64>,
        errors: u64,
    ) -> Result<(), String> {
        if !self.sink.is_enabled() {
            return Ok(());
        }
        let status = if errors == 0 { "ok" } else { "error" };
        record_required(
            self.sink.as_ref(),
            MetricEvent::counter(
                MetricId::ErrorsTotal,
                errors as f64,
                MetricLabels::one(LabelKey::Layer, "L2")
                    .with(LabelKey::Stage, "core")
                    .with(LabelKey::Status, status),
            ),
        )?;
        for (name, id) in [
            ("helix_lost_total", MetricId::LostTotal),
            ("helix_duplicate_total", MetricId::DuplicateTotal),
            ("helix_order_violation_total", MetricId::OrderViolationTotal),
            ("helix_wrong_channel_total", MetricId::WrongChannelTotal),
            (
                "helix_cursor_regression_total",
                MetricId::CursorRegressionTotal,
            ),
            (
                "helix_projection_mismatch_total",
                MetricId::ProjectionMismatchTotal,
            ),
            (
                "helix_persist_remaining_total",
                MetricId::PersistRemainingTotal,
            ),
            ("helix_cleanup_residue_total", MetricId::CleanupResidueTotal),
        ] {
            record_required(
                self.sink.as_ref(),
                MetricEvent::counter(
                    id,
                    correctness.get(name).copied().unwrap_or_default() as f64,
                    MetricLabels::one(LabelKey::Layer, "L2")
                        .with(LabelKey::Stage, "core")
                        .with(LabelKey::Status, "ok"),
                ),
            )?;
        }
        for id in [
            MetricId::EventLaggedTotal,
            MetricId::EventNoReceiverTotal,
            MetricId::FfiBusyTotal,
        ] {
            record_required(
                self.sink.as_ref(),
                MetricEvent::counter(
                    id,
                    0.0,
                    MetricLabels::one(LabelKey::Layer, "L2")
                        .with(LabelKey::Stage, "core")
                        .with(LabelKey::Status, "ok"),
                ),
            )?;
        }
        Ok(())
    }

    pub fn record_duration_histogram(
        &self,
        id: MetricId,
        histogram: &hdrhistogram::Histogram<u64>,
        stage: &'static str,
    ) -> Result<(), String> {
        if !self.sink.is_enabled() {
            return Ok(());
        }
        let labels = MetricLabels::one(LabelKey::Layer, "L2")
            .with(LabelKey::Stage, stage)
            .with(LabelKey::Status, "ok");
        for value in histogram.iter_recorded() {
            let seconds = value.value_iterated_to() as f64 / 1_000_000.0;
            for _ in 0..value.count_since_last_iteration() {
                record_required(
                    self.sink.as_ref(),
                    MetricEvent::histogram(id, seconds, labels),
                )?;
            }
        }
        Ok(())
    }

    pub fn shutdown(mut self) -> (ProbeSnapshot, bool) {
        let snapshot = self.snapshot();
        let drained = self.runtime.take().is_some_and(|runtime| {
            matches!(
                runtime.shutdown(Duration::from_secs(1)),
                helix_driver_host::metrics::ShutdownOutcome::Drained
                    | helix_driver_host::metrics::ShutdownOutcome::Disabled
            )
        });
        (snapshot, drained)
    }
}

#[cfg(feature = "sdk-metrics")]
fn record_required(sink: &dyn AsyncMetricSink, event: MetricEvent) -> Result<(), String> {
    match sink.try_record(event) {
        helix_driver_host::metrics::RecordOutcome::Accepted => Ok(()),
        outcome => Err(format!(
            "PERF_TELEMETRY_UNAVAILABLE: 最终正确性指标入队失败: {outcome:?}"
        )),
    }
}

#[cfg(feature = "sdk-metrics")]
struct ProbeExporter;

#[cfg(feature = "sdk-metrics")]
impl BatchMetricExporter for ProbeExporter {
    fn export_batch(&mut self, _batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        std::thread::sleep(Duration::from_millis(250));
        Ok(())
    }
}
