use std::sync::Arc;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::Tick;
use tokio::sync::mpsc;

use crate::network::{ConnectionEpochs, InboundTickSender, WsConnectionLifecycle};

/// 被替换的物理连接收尾时不得发布 `Disconnected`。
///
/// 线上 1:1 模式：2158 次 reconnect_success 对 2113 次 disconnect，服务端从未主动关闭。
/// 若被替换连接的收尾也能发布，supervisor 就会把「上一条连接没了」当成本次连接断了，
/// 每次重建都触发下一次重建，形成 60~80 次/分的自持风暴。
#[tokio::test]
async fn superseded_connection_close_does_not_publish_disconnected() {
    let (tick_tx, mut tick_rx) = mpsc::channel(8);
    let inbound = Some((TransportId::from_raw(9), InboundTickSender::Raw(tick_tx)));
    let epochs = Arc::new(ConnectionEpochs::default());

    let first = Arc::new(WsConnectionLifecycle::new_with_epoch(
        inbound.clone(),
        Some((epochs.next(), Arc::clone(&epochs))),
    ));
    assert!(first.activate().await.expect("activate first"));

    // 重建：第二代连接宣告自己是当前代，第一代随即成为被替换连接。
    let second = Arc::new(WsConnectionLifecycle::new_with_epoch(
        inbound.clone(),
        Some((epochs.next(), Arc::clone(&epochs))),
    ));
    assert!(second.activate().await.expect("activate second"));

    for _ in 0..2 {
        let tick = tokio::time::timeout(Duration::from_millis(200), tick_rx.recv())
            .await
            .expect("connected tick")
            .expect("channel open");
        assert!(matches!(tick, Tick::Connected(_)));
    }

    first.finish_once().await;
    let leaked = tokio::time::timeout(Duration::from_millis(100), tick_rx.recv()).await;
    assert!(
        leaked.is_err(),
        "被替换连接的 Disconnected 不得发布，实际收到 {leaked:?}"
    );

    second.finish_once().await;
    let current = tokio::time::timeout(Duration::from_millis(200), tick_rx.recv())
        .await
        .expect("当前代关闭必须发布 Disconnected")
        .expect("channel open");
    assert!(matches!(current, Tick::Disconnected(_)));
}

/// 守卫：没有代际语义的路径（FFI / Web / 单测直连）必须保持原有语义，收尾即发布。
#[tokio::test]
async fn connection_without_epoch_still_publishes_disconnected() {
    let (tick_tx, mut tick_rx) = mpsc::channel(8);
    let inbound = Some((TransportId::from_raw(10), InboundTickSender::Raw(tick_tx)));
    let lifecycle = Arc::new(WsConnectionLifecycle::new_with_epoch(inbound, None));

    assert!(lifecycle.activate().await.expect("activate"));
    lifecycle.finish_once().await;

    assert!(matches!(tick_rx.recv().await, Some(Tick::Connected(_))));
    let disconnected = tokio::time::timeout(Duration::from_millis(200), tick_rx.recv())
        .await
        .expect("无代际路径必须照旧发布 Disconnected");
    assert!(matches!(disconnected, Some(Tick::Disconnected(_))));
}

#[tokio::test]
async fn full_tick_queue_backpressures_without_losing_disconnect_notification() {
    let (tick_tx, mut tick_rx) = mpsc::channel(1);
    let inbound = Some((TransportId::from_raw(9), InboundTickSender::Raw(tick_tx)));
    let lifecycle = Arc::new(WsConnectionLifecycle::new_with_epoch(inbound, None));

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
