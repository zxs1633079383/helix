use super::MetricEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordOutcome {
    Accepted,
    Disabled,
    DroppedQueueFull,
}

pub trait AsyncMetricSink: Send + Sync {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome;

    fn is_enabled(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopMetricSink;

impl AsyncMetricSink for NoopMetricSink {
    #[inline]
    fn try_record(&self, _event: MetricEvent) -> RecordOutcome {
        RecordOutcome::Disabled
    }

    fn is_enabled(&self) -> bool {
        false
    }
}
