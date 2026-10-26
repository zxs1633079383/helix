use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

/// Locks down initial + three fast retries, followed by capped low-frequency recovery.
#[test]
fn three_fast_retries_then_background_is_unbounded_and_capped() {
    let policy = NetworkRecoveryPolicy::default();
    assert_eq!(
        (0..9)
            .map(|n| policy.delay_after_failures(n).as_secs())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 4, 15, 30, 60, 60, 60]
    );
    assert_eq!(
        policy.delay_after_failures(u32::MAX),
        Duration::from_secs(60)
    );
}

/// Offline cold startup spends no attempts; an online event interrupts background waiting.
#[tokio::test]
async fn offline_boot_and_online_wakeup_preserve_one_recovery_owner() {
    let id = TransportId::from_raw(9);
    let (network_tx, network_rx) = watch::channel(NetworkAvailability::Offline);
    let (stop_tx, stop_rx) = watch::channel(false);
    let (_events_tx, events_rx) = mpsc::unbounded_channel();
    let (attempt_tx, mut attempt_rx) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let policy = NetworkRecoveryPolicy {
        fast: ReconnectPolicy {
            initial_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(4),
            multiplier: 2,
            max_attempts: Some(3),
        },
        background: ReconnectPolicy {
            initial_delay: Duration::from_secs(30),
            max_delay: Duration::from_secs(60),
            multiplier: 2,
            max_attempts: None,
        },
    };
    let task = spawn_network_recovery(
        id,
        NetworkRecovery {
            availability: network_rx,
            policy,
        },
        true,
        events_rx,
        stop_rx,
        ReconnectTraceSink::default(),
        move || {
            let counter = counter.clone();
            let attempt_tx = attempt_tx.clone();
            async move {
                let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
                attempt_tx.send(n).unwrap();
                Err(PortError::Transport("unavailable".into()))
            }
        },
        || async {},
    );
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
    network_tx.send_replace(NetworkAvailability::Online);
    for expected in 1..=4 {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), attempt_rx.recv())
                .await
                .unwrap(),
            Some(expected)
        );
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        4,
        "three extra fast attempts then background wait"
    );
    network_tx.send_replace(NetworkAvailability::Offline);
    tokio::time::sleep(Duration::from_millis(10)).await;
    network_tx.send_replace(NetworkAvailability::Online);
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(200), attempt_rx.recv())
            .await
            .unwrap(),
        Some(5)
    );
    stop_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}

/// A positive network hint during a stalled handshake must not bypass shutdown cancellation.
#[tokio::test]
async fn shutdown_cancels_handshake_after_online_hint() {
    let (network_tx, network_rx) = watch::channel(NetworkAvailability::Unknown);
    let (stop_tx, stop_rx) = watch::channel(false);
    let (_event_tx, event_rx) = mpsc::unbounded_channel();
    let (started_tx, mut started_rx) = mpsc::unbounded_channel();
    let task = spawn_network_recovery(
        TransportId::from_raw(1),
        NetworkRecovery {
            availability: network_rx,
            policy: NetworkRecoveryPolicy::default(),
        },
        true,
        event_rx,
        stop_rx,
        ReconnectTraceSink::default(),
        move || {
            let tx = started_tx.clone();
            async move {
                tx.send(()).unwrap();
                std::future::pending::<Result<(), PortError>>().await
            }
        },
        || async {},
    );
    tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
        .await
        .unwrap();
    network_tx.send_replace(NetworkAvailability::Online);
    tokio::time::sleep(Duration::from_millis(10)).await;
    stop_tx.send_replace(true);
    tokio::time::timeout(Duration::from_millis(200), task)
        .await
        .unwrap()
        .unwrap();
}

/// A local disconnect closes a stale socket once; repeated online hints do not duplicate recovery.
#[tokio::test]
async fn established_connection_closes_on_offline_and_recovers_once() {
    let (network_tx, rx) = watch::channel(NetworkAvailability::Online);
    let (stop_tx, stop_rx) = watch::channel(false);
    let (_event_tx, events) = mpsc::unbounded_channel();
    let (connected_tx, mut connected_rx) = mpsc::unbounded_channel();
    let closes = Arc::new(AtomicUsize::new(0));
    let close_counter = closes.clone();
    let task = spawn_network_recovery(
        TransportId::from_raw(4),
        NetworkRecovery {
            availability: rx,
            policy: NetworkRecoveryPolicy::default(),
        },
        true,
        events,
        stop_rx,
        ReconnectTraceSink::default(),
        move || {
            let tx = connected_tx.clone();
            async move {
                tx.send(()).unwrap();
                Ok(())
            }
        },
        move || {
            let counter = close_counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
            }
        },
    );
    tokio::time::timeout(Duration::from_secs(1), connected_rx.recv())
        .await
        .unwrap();
    network_tx.send_replace(NetworkAvailability::Offline);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    assert!(connected_rx.try_recv().is_err());
    network_tx.send_replace(NetworkAvailability::Online);
    tokio::time::timeout(Duration::from_millis(200), connected_rx.recv())
        .await
        .unwrap();
    network_tx.send_replace(NetworkAvailability::Online);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(connected_rx.try_recv().is_err());
    assert_eq!(closes.load(Ordering::SeqCst), 1);
    stop_tx.send_replace(true);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap();
}
