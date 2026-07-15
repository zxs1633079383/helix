use std::sync::Arc;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::Tick;
use tokio::sync::mpsc;

use crate::network::{InboundTickSender, WsConnectionLifecycle};

#[tokio::test]
async fn full_tick_queue_backpressures_without_losing_disconnect_notification() {
    let (tick_tx, mut tick_rx) = mpsc::channel(1);
    let inbound = Some((TransportId::from_raw(9), InboundTickSender::Raw(tick_tx)));
    let lifecycle = Arc::new(WsConnectionLifecycle::new(inbound));

    assert!(lifecycle.activate().await.expect("activate"));
    let finishing = {
        let lifecycle = Arc::clone(&lifecycle);
        tokio::spawn(async move { lifecycle.finish_once().await })
    };

    assert!(matches!(tick_rx.recv().await, Some(Tick::Connected(_))));
    let disconnected = tokio::time::timeout(Duration::from_millis(200), tick_rx.recv())
        .await
        .expect("disconnect should deliver after capacity returns");
    assert!(matches!(disconnected, Some(Tick::Disconnected(_))));
    finishing.await.expect("finish task");
}
