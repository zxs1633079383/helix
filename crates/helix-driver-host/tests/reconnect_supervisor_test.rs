use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::PortError;
use helix_driver_host::engine::{
    TransportLifecycleEvent, TransportTraceEvent, TransportTraceSink,
    TRANSPORT_TRACE_QUEUE_CAPACITY,
};
use helix_driver_host::{spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink};
use tokio::sync::{mpsc, watch};

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
    let (tx, mut rx) = TransportTraceSink::channel();
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

#[test]
fn transport_trace_queue_is_bounded_and_counts_drops() {
    let (sink, _rx) = TransportTraceSink::channel();
    for _ in 0..TRANSPORT_TRACE_QUEUE_CAPACITY {
        assert!(sink.try_emit(trace_event("fill")));
    }
    assert!(!sink.try_emit(trace_event("overflow")));
    assert_eq!(sink.stats().dropped_count(), 1);
}

#[tokio::test]
async fn reconnect_supervisor_reports_success_after_full_action() {
    let transport_id = TransportId::from_raw(11);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(50),
        multiplier: 2,
        max_attempts: Some(3),
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let (completed_tx, mut completed_rx) = mpsc::unbounded_channel();
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);

    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        shutdown_rx,
        ReconnectTraceSink::new(Some(trace_tx)),
        move || {
            let attempts = Arc::clone(&attempts_for_factory);
            let completed_tx = completed_tx.clone();
            async move {
                let current = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                if current == 1 {
                    Err(PortError::Transport(
                        "token=secret raw websocket failure body message".to_string(),
                    ))
                } else {
                    completed_tx
                        .send(())
                        .map_err(|_| PortError::Transport("completion channel closed".into()))?;
                    Ok(())
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

    tokio::time::timeout(Duration::from_secs(1), completed_rx.recv())
        .await
        .expect("complete reconnect action should finish")
        .expect("completion channel should stay open");

    let events = collect_trace_events(&mut trace_rx, 6).await;
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

#[tokio::test]
async fn reconnect_supervisor_shutdown_interrupts_pending_attempt() {
    let transport_id = TransportId::from_raw(12);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
        multiplier: 1,
        max_attempts: None,
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (attempt_tx, mut attempt_rx) = mpsc::unbounded_channel();
    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        shutdown_rx,
        ReconnectTraceSink::default(),
        move || {
            let attempt_tx = attempt_tx.clone();
            async move {
                attempt_tx.send(()).ok();
                std::future::pending::<Result<(), PortError>>().await
            }
        },
    );

    lifecycle_tx
        .send(TransportLifecycleEvent::Disconnected {
            transport_id,
            reason: "reader_closed_or_error",
        })
        .expect("send lifecycle event");
    tokio::time::timeout(Duration::from_secs(1), attempt_rx.recv())
        .await
        .expect("first reconnect attempt should run")
        .expect("attempt channel should stay open");

    shutdown_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("shutdown should interrupt a pending reconnect action")
        .expect("supervisor task should not panic");
    drop(lifecycle_tx);
}

#[tokio::test]
async fn reconnect_supervisor_shutdown_interrupts_backoff() {
    let transport_id = TransportId::from_raw(13);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_secs(60),
        max_delay: Duration::from_secs(60),
        multiplier: 1,
        max_attempts: None,
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();
    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        shutdown_rx,
        ReconnectTraceSink::new(Some(trace_tx)),
        || async { Err(PortError::Transport("must not run".to_string())) },
    );

    lifecycle_tx
        .send(TransportLifecycleEvent::Disconnected {
            transport_id,
            reason: "reader_closed_or_error",
        })
        .expect("send lifecycle event");
    let scheduled = tokio::time::timeout(Duration::from_secs(1), trace_rx.recv())
        .await
        .expect("backoff schedule should be emitted")
        .expect("trace channel should stay open");
    assert_eq!(scheduled.action, "reconnect_schedule");

    shutdown_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), handle)
        .await
        .expect("shutdown should interrupt a long backoff")
        .expect("supervisor task should not panic");
    drop(lifecycle_tx);
}

/// 线上 1:1 模式复现：客户端每次「连接成功」后，上一条连接迟到的 Disconnected 紧随其后到达
/// （10 小时 2158 次 reconnect_success 对 2113 次 disconnect，服务端日志证明它从未主动关闭）。
///
/// 不变式：成功后极短时间内又被告知断开＝「连上即断」抖动，退避必须继续升级并保持有界；
/// 现状是每条 Disconnected 都把 attempt 重置为 1，于是永远在 initial_delay 上打转，
/// 也就是线上 60~70 次/分的连接风暴。
#[tokio::test(start_paused = true)]
async fn replacement_disconnect_after_success_must_escalate_backoff() {
    let transport_id = TransportId::from_raw(21);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(200),
        max_delay: Duration::from_secs(10),
        multiplier: 2,
        max_attempts: None,
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_factory = Arc::clone(&attempts);
    let lifecycle_for_factory = lifecycle_tx.clone();

    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        shutdown_rx,
        ReconnectTraceSink::new(Some(trace_tx)),
        move || {
            let attempts = Arc::clone(&attempts_for_factory);
            let lifecycle_tx = lifecycle_for_factory.clone();
            async move {
                attempts.fetch_add(1, Ordering::SeqCst);
                // 本次重连成功的同时，上一条连接（同一 transport_id）的 Disconnected 才被投递。
                lifecycle_tx
                    .send(TransportLifecycleEvent::Disconnected {
                        transport_id,
                        reason: "reader_closed_or_error",
                    })
                    .ok();
                Ok(())
            }
        },
    );

    lifecycle_tx
        .send(TransportLifecycleEvent::Disconnected {
            transport_id,
            reason: "reader_closed_or_error",
        })
        .expect("send lifecycle event");

    tokio::time::sleep(Duration::from_secs(2)).await;
    let attempts_in_two_seconds = attempts.load(Ordering::SeqCst);

    shutdown_tx.send_replace(true);
    handle.await.expect("supervisor task should not panic");

    let mut scheduled_attempts = Vec::new();
    while let Ok(event) = trace_rx.try_recv() {
        if event.action == "reconnect_schedule" {
            scheduled_attempts.push(event.attempt.unwrap_or(0));
        }
    }

    assert!(
        attempts_in_two_seconds <= 4,
        "2s 内只允许 4 次以内重建（200/400/800/1600ms 退避），实际 {attempts_in_two_seconds} 次；\
         调度 attempt 序列 {scheduled_attempts:?}"
    );
    assert!(
        scheduled_attempts.len() >= 2 && scheduled_attempts[1] > scheduled_attempts[0],
        "连上即断后必须继续升级退避，实际调度 attempt 序列 {scheduled_attempts:?}"
    );
}

/// 反向守卫：连接稳定存活超过抖动窗口后再断开，属于新故障，退避必须回到首档。
/// 没有这条，抖动识别会退化成「重连一次比一次慢」，稳定网络下的快速恢复也会被拖死。
#[tokio::test(start_paused = true)]
async fn stable_connection_resets_backoff_to_initial_delay() {
    let transport_id = TransportId::from_raw(22);
    let policy = ReconnectPolicy {
        initial_delay: Duration::from_millis(200),
        max_delay: Duration::from_secs(10),
        multiplier: 2,
        max_attempts: None,
    };
    let (lifecycle_tx, lifecycle_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (trace_tx, mut trace_rx) = TransportTraceSink::channel();

    let handle = spawn_reconnect_supervisor(
        transport_id,
        policy,
        lifecycle_rx,
        shutdown_rx,
        ReconnectTraceSink::new(Some(trace_tx)),
        || async { Ok(()) },
    );

    let disconnected = || TransportLifecycleEvent::Disconnected {
        transport_id,
        reason: "reader_closed_or_error",
    };

    lifecycle_tx.send(disconnected()).expect("first disconnect");
    // 让首轮重连完成，并让连接稳定存活过抖动窗口。
    tokio::time::sleep(Duration::from_secs(6)).await;
    lifecycle_tx
        .send(disconnected())
        .expect("second disconnect");
    tokio::time::sleep(Duration::from_millis(500)).await;

    shutdown_tx.send_replace(true);
    handle.await.expect("supervisor task should not panic");

    let mut schedules = Vec::new();
    while let Ok(event) = trace_rx.try_recv() {
        if event.action == "reconnect_schedule" {
            schedules.push((event.attempt.unwrap_or(0), event.delay_ms.unwrap_or(0)));
        }
    }

    assert_eq!(
        schedules,
        vec![(1, 200), (1, 200)],
        "稳定连接后的再次断开必须从首档 200ms 重来，实际 {schedules:?}"
    );
}

async fn collect_trace_events(
    rx: &mut mpsc::Receiver<TransportTraceEvent>,
    expected: usize,
) -> Vec<TransportTraceEvent> {
    let mut events = Vec::new();
    while events.len() < expected {
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("trace event should arrive")
            .expect("trace channel should stay open");
        events.push(event);
    }
    events
}

fn trace_event(action: &'static str) -> TransportTraceEvent {
    TransportTraceEvent {
        transport_id: TransportId::from_raw(1),
        name: "helix.ws.test",
        action,
        attempt: None,
        delay_ms: None,
        next_delay_ms: None,
        reason: None,
        error_class: None,
    }
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
