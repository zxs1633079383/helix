//! 通用宿主只接收低基数标量，不反向依赖IM业务crate。
use super::{AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels};

/// 装配层传递既有业务终态；宿主负责非阻塞记录，拒绝未知结果标签。
pub fn record_sync_timing(
    sink: &dyn AsyncMetricSink,
    recovery: bool,
    result: &'static str,
    started_at_ms: u64,
    completed_at_ms: u64,
    total_pages: Option<u64>,
) {
    if !matches!(result, "started" | "success" | "failed" | "cancelled") {
        return;
    }
    if !sink.is_enabled() {
        return;
    }
    let labels = MetricLabels::one(LabelKey::Result, result);
    let elapsed = completed_at_ms.saturating_sub(started_at_ms) as f64 / 1000.0;
    if result == "started" {
        if recovery {
            let _ = sink.try_record(MetricEvent::counter(
                MetricId::RecoveryStartedTotal,
                1.0,
                MetricLabels::default(),
            ));
            let _ = sink.try_record(MetricEvent::gauge(
                MetricId::RecoveryStartedTimestampSeconds,
                started_at_ms as f64 / 1000.0,
                MetricLabels::default(),
            ));
        }
        return;
    }
    let (counter, duration) = match recovery {
        true => (
            MetricId::RecoveryTerminalTotal,
            MetricId::RecoveryDurationSeconds,
        ),
        false => (
            MetricId::IncrementPageTerminalTotal,
            MetricId::IncrementPageDurationSeconds,
        ),
    };
    let _ = sink.try_record(MetricEvent::counter(counter, 1.0, labels));
    let _ = sink.try_record(MetricEvent::histogram(duration, elapsed, labels));
    if recovery {
        if let Some(pages) = total_pages {
            let _ = sink.try_record(MetricEvent::histogram(
                MetricId::RecoveryInventoryPages,
                pages as f64,
                labels,
            ));
        }
        if result == "success" {
            let _ = sink.try_record(MetricEvent::gauge(
                MetricId::RecoveryCompletedTimestampSeconds,
                completed_at_ms as f64 / 1000.0,
                MetricLabels::default(),
            ));
        }
    }
}
