use std::sync::Mutex;

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_driver_host::{
    record_im_business_event, AsyncMetricSink, LabelKey, MetricEvent, MetricId, RecordOutcome,
};

#[derive(Default)]
struct RecordingMetrics {
    events: Mutex<Vec<MetricEvent>>,
}

impl AsyncMetricSink for RecordingMetrics {
    /// 记录测试观察到的业务样本，不引入异步导出行为。
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.events.lock().expect("metrics lock").push(event);
        RecordOutcome::Accepted
    }
}

/// event_bytes 构造 canonical MessageV3 信封，保持生产分类器的真实字节前缀。
fn event_bytes(event: &'static str) -> DomainEventBytes {
    DomainEventBytes(Bytes::from(format!(r#"{{"event":"{event}","data":{{}}}}"#)))
}

/// has_event 按 MetricId 和完整低基数 labels 匹配一个观测样本。
fn has_event(sink: &RecordingMetrics, id: MetricId, labels: &[(LabelKey, &str)]) -> bool {
    sink.events
        .lock()
        .expect("metrics lock")
        .iter()
        .any(|event| {
            event.id == id
                && labels.iter().all(|(key, value)| {
                    event
                        .labels
                        .iter()
                        .any(|label| label.key == *key && label.value == *value)
                })
        })
}

/// post_received_records_projection_without_assuming_command_lineage 锁定远端推送只算权威投影。
#[test]
fn post_received_records_projection_without_assuming_command_lineage() {
    let sink = RecordingMetrics::default();
    record_im_business_event(&sink, &event_bytes("im:post:received"));

    assert!(has_event(
        &sink,
        MetricId::ImProjectionTerminalTotal,
        &[
            (LabelKey::Operation, "send_message"),
            (LabelKey::Status, "success"),
        ],
    ));
    assert!(!sink
        .events
        .lock()
        .expect("metrics lock")
        .iter()
        .any(|event| event.id == MetricId::ImCommandTerminalTotal));
    assert!(has_event(
        &sink,
        MetricId::ImMessageCorrectnessTotal,
        &[
            (LabelKey::Operation, "message_projection"),
            (LabelKey::Status, "success"),
        ],
    ));
}

/// sync_events_record_one_session_terminal_and_bounded_anomaly_kind 验证成功与 TooLong 不混口径。
#[test]
fn sync_events_record_one_session_terminal_and_bounded_anomaly_kind() {
    let sink = RecordingMetrics::default();
    record_im_business_event(&sink, &event_bytes("im:channel-sync-complete"));
    record_im_business_event(&sink, &event_bytes("im:sync:too_long"));

    assert!(has_event(
        &sink,
        MetricId::ImSyncSessionTotal,
        &[
            (LabelKey::Operation, "channel_sync"),
            (LabelKey::Status, "success"),
        ],
    ));
    assert!(has_event(
        &sink,
        MetricId::ImSyncSessionTotal,
        &[
            (LabelKey::Operation, "channel_sync"),
            (LabelKey::Status, "error"),
            (LabelKey::ErrorKind, "too_long"),
        ],
    ));
    assert!(has_event(
        &sink,
        MetricId::ImSyncAnomalyTotal,
        &[(LabelKey::ErrorKind, "too_long")],
    ));
    assert!(has_event(
        &sink,
        MetricId::ImMessageCorrectnessTotal,
        &[(LabelKey::Status, "error"), (LabelKey::ErrorKind, "gap"),],
    ));
}

/// recovery_event_records_real_terminal 验证 recovered 只在稳定恢复事件处计成功。
#[test]
fn recovery_event_records_real_terminal() {
    let sink = RecordingMetrics::default();
    record_im_business_event(&sink, &event_bytes("im:sync:recovered"));

    assert!(has_event(
        &sink,
        MetricId::ImRecoverySessionTotal,
        &[
            (LabelKey::Operation, "offline_recovery"),
            (LabelKey::Status, "success"),
        ],
    ));
}

/// client_ack_events_record_transport_and_business_terminal 锁定 ACK 成败来自真实 HTTP 回报。
#[test]
fn client_ack_events_record_transport_and_business_terminal() {
    let sink = RecordingMetrics::default();
    record_im_business_event(&sink, &event_bytes("im:post:client-ack-succeeded"));
    record_im_business_event(&sink, &event_bytes("im:post:client-ack-failed"));

    assert!(has_event(
        &sink,
        MetricId::ImClientAckTerminalTotal,
        &[(LabelKey::Status, "success")],
    ));
    assert!(has_event(
        &sink,
        MetricId::ImClientAckTerminalTotal,
        &[(LabelKey::Status, "error")],
    ));
}

/// intermediate_and_unknown_events_do_not_create_false_terminals 防止 ready/page/未知事件污染分母。
#[test]
fn intermediate_and_unknown_events_do_not_create_false_terminals() {
    let sink = RecordingMetrics::default();
    for event in [
        "im:post:sending",
        "im:channel-sync-ready",
        "im:channel-sync-page",
        "im:unknown:event",
    ] {
        record_im_business_event(&sink, &event_bytes(event));
    }

    assert!(sink.events.lock().expect("metrics lock").is_empty());
}

/// unread_reconcile_records_only_explicit_authority_comparison 验证 durable read-back 携带的双侧绝对值才进入指标。
#[test]
fn unread_reconcile_records_only_explicit_authority_comparison() {
    let sink = RecordingMetrics::default();
    let event = DomainEventBytes(Bytes::from_static(
        br#"{"event":"im:read:result","data":{"body":{"unreadReconcile":{"authority":3,"local":3,"status":"match"}}}}"#,
    ));
    record_im_business_event(&sink, &event);

    assert_eq!(
        MetricId::ImUnreadReconcileTotal.availability(),
        helix_driver_host::MetricAvailability::Runtime,
    );
    assert!(has_event(
        &sink,
        MetricId::ImUnreadReconcileTotal,
        &[(LabelKey::Status, "match")],
    ));
}
