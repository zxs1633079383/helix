use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::FrameSender;
use helix_core::PortError;
use helix_driver_host::engine::{TransportLifecycleEvent, TransportTraceEvent};
use helix_driver_host::{spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink};
use tokio::sync::mpsc;

#[test]
fn reconnect_policy_uses_capped_exponential_delays() {
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(200),
        max_delay: Duration::from_secs(10),
        multiplier: 2,
        max_attempts: None,
    };

    assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(200));
    assert_eq!(policy.delay_for_attempt(2), Duration::from_millis(400));
    assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(800));
    assert_eq!(policy.delay_for_attempt(10), Duration::from_secs(10));
}

#[test]
fn reconnect_trace_sink_emits_safe_payload_shape() {
    let (tx, mut rx) = mpsc::unbounded_channel::<TransportTraceEvent>();
    let sink = ReconnectTraceSink::new(Some(tx));

    sink.emit(TransportTraceEvent {
        transport_id: TransportId::from_raw(7),
        name: "helix.ws.reconnect.schedule",
        action: "reconnect_schedule",
        attempt: Some(2),
        delay_ms: Some(400),
        next_delay_ms: None,
        reason: Some("reader_closed_or_error"),
        error_class: None,
    });

    let event = rx.try_recv().expect("trace event");
    assert_eq!(event.transport_id, TransportId::from_raw(7));
    assert_eq!(event.name, "helix.ws.reconnect.schedule");
    assert_eq!(event.action, "reconnect_schedule");
    assert_eq!(event.attempt, Some(2));
    assert_eq!(event.delay_ms, Some(400));
    assert_eq!(event.next_delay_ms, None);
    assert_eq!(event.reason, Some("reader_closed_or_error"));
    assert!(event.error_class.is_none());
}

#[tokio::test]
async fn reconnect_supervisor_registers_transport_after_retry_success() {
    let transport_id = TransportId::from_raw(11);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(50),
        multiplier: 2,
        max_attempts: Some(3),
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (register_tx, mut register_rx) = mpsc::unbounded_channel();
    let (trace_tx, mut trace_rx) = mpsc::unbounded_channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);

    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        register_tx,
        ReconnectTraceSink::new(Some(trace_tx)),
        move || {
            let attempts = Arc::clone(&attempts_for_factory);
            async move {
                let current = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if current == 1 {
                    Err(PortError::Transport(
                        "token=secret raw websocket failure body message".to_string(),
                    ))
                } else {
                    Ok(TestTransport)
                }
            }
        },
    );

    lifecycle_tx
        .send(TransportLifecycleEvent::Disconnected {
            transport_id,
            reason: "reader_closed_or_error",
        })
        .expect("send lifecycle event");

    let registered = tokio::time::timeout(Duration::from_secs(1), register_rx.recv())
        .await
        .expect("registered transport should arrive")
        .expect("register channel should stay open");
    assert_eq!(registered.0, transport_id);

    let events = drain_trace_events(&mut trace_rx);
    let actions: Vec<&'static str> = events.iter().map(|event| event.action).collect();
    assert_eq!(
        actions,
        vec![
            "reconnect_schedule",
            "reconnect_attempt",
            "reconnect_failed",
            "reconnect_schedule",
            "reconnect_attempt",
            "reconnect_success",
        ]
    );
    assert_eq!(events[0].attempt, Some(1));
    assert_eq!(events[0].delay_ms, Some(10));
    assert_eq!(events[0].reason, Some("reader_closed_or_error"));
    assert_eq!(events[1].delay_ms, Some(10));
    assert_eq!(events[2].error_class.as_deref(), Some("transport"));
    assert_eq!(events[2].next_delay_ms, Some(20));
    assert_eq!(events[3].attempt, Some(2));
    assert_eq!(events[3].delay_ms, Some(20));
    assert_eq!(events[4].delay_ms, Some(20));
    assert!(events.iter().all(trace_event_payload_is_safe));

    drop(lifecycle_tx);
    handle
        .await
        .expect("supervisor should stop after channel closes");
}

fn drain_trace_events(
    rx: &mut mpsc::UnboundedReceiver<TransportTraceEvent>,
) -> Vec<TransportTraceEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn trace_event_payload_is_safe(event: &TransportTraceEvent) -> bool {
    let text = format!(
        "{} {} {:?} {:?}",
        event.name, event.action, event.reason, event.error_class
    );
    ![
        "headers",
        "cookieId",
        "companyId",
        "token",
        "body",
        "message",
        "text",
        "secret",
    ]
    .iter()
    .any(|forbidden| text.contains(forbidden))
}

struct TestTransport;

#[async_trait::async_trait]
impl FrameSender for TestTransport {
    async fn send(&self, _: Bytes) -> Result<(), PortError> {
        Ok(())
    }
}
