#![cfg(feature = "sdk-metrics")]

use std::sync::{Arc, Mutex};

use hdrhistogram::Histogram;
use helix_driver_host::metrics::{AsyncMetricSink, MetricEvent, MetricId, RecordOutcome};

pub type QueueWaitCapture = Arc<Mutex<Histogram<u64>>>;

struct CapturingMetricSink {
    inner: Arc<dyn AsyncMetricSink>,
    queue_wait: QueueWaitCapture,
}

pub fn wrap(inner: Arc<dyn AsyncMetricSink>) -> (Arc<dyn AsyncMetricSink>, QueueWaitCapture) {
    let queue_wait = Arc::new(Mutex::new(
        Histogram::new(3).expect("queue wait capture histogram"),
    ));
    let sink: Arc<dyn AsyncMetricSink> = Arc::new(CapturingMetricSink {
        inner,
        queue_wait: Arc::clone(&queue_wait),
    });
    (sink, queue_wait)
}

impl AsyncMetricSink for CapturingMetricSink {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        if event.id == MetricId::TickQueueWaitSeconds {
            let micros = (event.value * 1_000_000.0).max(1.0) as u64;
            let _ = self
                .queue_wait
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .record(micros);
        }
        self.inner.try_record(event)
    }

    fn is_enabled(&self) -> bool {
        self.inner.is_enabled()
    }
}

pub fn snapshot(capture: &QueueWaitCapture) -> Histogram<u64> {
    capture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}
