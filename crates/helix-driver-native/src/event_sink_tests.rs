use super::*;
use bytes::Bytes;
use helix_driver_host::RecordOutcome;
use std::sync::Mutex;

#[derive(Default)]
struct RecordingMetrics(Mutex<Vec<MetricEvent>>);

impl AsyncMetricSink for RecordingMetrics {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

#[tokio::test]
async fn test_emit_received_by_subscriber() {
    let (sink, mut rx) = NativeEventSink::new();
    let payload = Bytes::from_static(b"hello-event");
    sink.emit(DomainEventBytes(payload.clone()));
    assert_eq!(rx.recv().await.expect("should receive event").0, payload);
}

#[tokio::test]
async fn test_emit_no_receiver_does_not_panic() {
    let (sink, rx) = NativeEventSink::new();
    drop(rx);
    sink.emit(DomainEventBytes(Bytes::from_static(b"orphan")));
}

#[tokio::test]
async fn no_receiver_records_metric() {
    let metrics = Arc::new(RecordingMetrics::default());
    let (sink, rx) = NativeEventSink::new();
    let sink = sink.with_metric_sink(metrics.clone());
    drop(rx);
    sink.emit(DomainEventBytes(Bytes::from_static(b"orphan")));
    assert_eq!(
        metrics.0.lock().unwrap()[0].id,
        MetricId::EventNoReceiverTotal
    );
}

#[tokio::test]
async fn test_lagged_surfaced_not_silent_e7() {
    let (sink, rx) = NativeEventSink::new_with_capacity(2);
    let mut obs = EventReceiver::new(rx);
    for i in 0..5u8 {
        sink.emit(DomainEventBytes(Bytes::from(vec![i])));
    }
    match obs.recv().await {
        RecvOutcome::Lagged(n) => {
            assert!(n >= 1);
            assert!(obs.lagged_total() >= 1);
        }
        other => panic!("落后时首个结果必须是 Lagged，实际 {other:?}"),
    }
    assert!(matches!(obs.recv().await, RecvOutcome::Event(_)));
}

#[tokio::test]
async fn lagged_records_metric() {
    let metrics = Arc::new(RecordingMetrics::default());
    let (sink, _rx) = NativeEventSink::new_with_capacity(2);
    let sink = sink.with_metric_sink(metrics.clone());
    let mut observed = sink.subscribe_observed();
    for i in 0..5u8 {
        sink.emit(DomainEventBytes(Bytes::from(vec![i])));
    }
    assert!(matches!(observed.recv().await, RecvOutcome::Lagged(_)));
    assert!(metrics
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.id == MetricId::EventLaggedTotal));
}

/// observed receiver 应携带 consumer 标签并发布 receiver Gauge。
#[tokio::test]
async fn observed_receiver_records_bounded_consumer_identity() {
    let metrics = Arc::new(RecordingMetrics::default());
    let (sink, _rx) = NativeEventSink::new();
    let sink = sink.with_metric_sink(metrics.clone());
    let _observed = sink.subscribe_observed_as("tauri_bridge");

    let events = metrics.0.lock().unwrap();
    let receiver = events
        .iter()
        .find(|event| event.id == MetricId::EventReceiverCount)
        .expect("订阅必须发布 receiver count");
    assert!(receiver
        .labels
        .iter()
        .any(|label| { label.key == LabelKey::Consumer && label.value == "tauri_bridge" }));
}

#[tokio::test]
async fn test_closed_outcome_e7() {
    let (sink, rx) = NativeEventSink::new();
    let mut obs = EventReceiver::new(rx);
    drop(sink);
    assert!(matches!(obs.recv().await, RecvOutcome::Closed));
}

#[tokio::test]
async fn test_multiple_subscribers() {
    let (sink, mut rx1) = NativeEventSink::new();
    let mut rx2 = sink.subscribe();
    let payload = Bytes::from_static(b"broadcast");
    sink.emit(DomainEventBytes(payload.clone()));
    assert_eq!(rx1.recv().await.expect("rx1").0, payload);
    assert_eq!(rx2.recv().await.expect("rx2").0, payload);
}

fn domain_event(json: serde_json::Value) -> DomainEventBytes {
    DomainEventBytes(Bytes::from(serde_json::to_vec(&json).unwrap()))
}

#[test]
fn to_bus_envelope_wraps_event_as_channel() {
    let ev = domain_event(serde_json::json!({
        "event": "im:post:read",
        "data": { "channel_id": "ch_1", "msg_id": "m_1" }
    }));
    let envelope = to_bus_envelope(&ev);
    assert_eq!(
        envelope.get("channel").and_then(|c| c.as_str()),
        Some("im:post:read")
    );
    assert_eq!(
        envelope.get("payload"),
        Some(&serde_json::json!({
            "event": "im:post:read",
            "data": { "channel_id": "ch_1", "msg_id": "m_1" }
        }))
    );
}

#[test]
fn to_bus_envelope_invalid_json_falls_back_empty_channel() {
    let envelope = to_bus_envelope(&DomainEventBytes(Bytes::from_static(b"not json{")));
    assert_eq!(envelope.get("channel").and_then(|c| c.as_str()), Some(""));
}

#[test]
fn bus_event_name_extracts_event_field() {
    let ev = domain_event(serde_json::json!({"event": "im:post:deleted", "data": {}}));
    assert_eq!(bus_event_name(&ev).as_deref(), Some("im:post:deleted"));
}

#[test]
fn bus_event_name_none_on_missing_or_invalid() {
    assert_eq!(
        bus_event_name(&domain_event(serde_json::json!({"x": 1}))),
        None
    );
    assert_eq!(
        bus_event_name(&DomainEventBytes(Bytes::from_static(b"{["))),
        None
    );
}

#[test]
fn bus_channel_constant_is_internal_namespace() {
    assert!(BUS_CHANNEL.starts_with("im:__"));
}
