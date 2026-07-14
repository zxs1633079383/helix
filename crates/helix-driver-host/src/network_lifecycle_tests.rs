use std::sync::atomic::AtomicBool;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::Tick;
use tokio::sync::mpsc;

use crate::network::notify_disconnected;

#[tokio::test]
async fn full_tick_queue_does_not_block_disconnect_notification() {
    let (tick_tx, mut tick_rx) = mpsc::channel(1);
    tick_tx
        .try_send(Tick::Connected(TransportId::from_raw(9)))
        .expect("fill tick queue");
    let inbound = Some((TransportId::from_raw(9), tick_tx));
    let notified = AtomicBool::new(false);

    notify_disconnected(&inbound, &notified);
    assert!(matches!(tick_rx.recv().await, Some(Tick::Connected(_))));
    let disconnected = tokio::time::timeout(Duration::from_millis(200), tick_rx.recv())
        .await
        .expect("spawned fallback should deliver after capacity returns");
    assert!(matches!(disconnected, Some(Tick::Disconnected(_))));
}
