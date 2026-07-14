use super::*;
use crate::metrics::NoopMetricSink;

#[test]
fn pending_port_correlation_is_strictly_bounded() {
    let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    for corr in 0..5000 {
        recorder.remember_port_start(corr);
    }
    assert_eq!(recorder.port_started.len(), MAX_PENDING_PORTS);
    assert_eq!(
        recorder.port_slots.iter().flatten().count(),
        MAX_PENDING_PORTS
    );
}

#[test]
fn insert_reply_cycles_do_not_accumulate_stale_order_entries() {
    let mut recorder = EngineMetricRecorder::new(Arc::new(NoopMetricSink));
    for corr in 0..(MAX_PENDING_PORTS as u64 * 3) {
        recorder.remember_port_start(corr);
        assert!(recorder.complete_port(corr).is_some());
    }
    assert!(recorder.port_started.is_empty());
    assert_eq!(recorder.port_slots.iter().flatten().count(), 0);
}
