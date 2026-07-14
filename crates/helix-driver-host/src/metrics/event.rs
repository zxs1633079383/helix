use super::{MetricId, MetricKind, MetricLabels};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricEvent {
    pub id: MetricId,
    pub value: f64,
    pub labels: MetricLabels,
}

impl MetricEvent {
    pub fn counter(id: MetricId, value: f64, labels: MetricLabels) -> Self {
        debug_assert_eq!(id.descriptor().kind, MetricKind::Counter);
        Self { id, value, labels }
    }

    pub fn histogram(id: MetricId, value: f64, labels: MetricLabels) -> Self {
        debug_assert_eq!(id.descriptor().kind, MetricKind::Histogram);
        Self { id, value, labels }
    }

    pub fn gauge(id: MetricId, value: f64, labels: MetricLabels) -> Self {
        debug_assert_eq!(id.descriptor().kind, MetricKind::Gauge);
        Self { id, value, labels }
    }
}
