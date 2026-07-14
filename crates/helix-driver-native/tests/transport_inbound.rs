//! A3 集成验证：平台连接生命周期 → 既有 Connected/Inbound/Disconnected Tick。
//!
//! 证明 sync-v2 入站半边真接通（能收 hello/increment 帧），而非仅编译通过。
//! 读任务独立持有 stream 半边，把数据帧作 `Tick::Inbound` 直接喂 tick_tx，
//! 与 `Tick::Connected`/`Disconnected` 同一通道（不碰 state Mutex，不与 send 抢锁）。

use std::time::Duration;

use std::sync::Arc;
use std::task::Poll;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use helix_core::effect::TransportId;
use helix_core::ports::FrameSender;
use helix_core::Tick;
use helix_driver_native::{NativeTransport, TransportTable};

const HELLO: &[u8] = b"{\"action\":\"hello\"}";
const PROBE: &[u8] = b"{\"action\":\"probe\"}";

#[tokio::test]
async fn connection_emits_connected_inbound_and_one_reader_disconnected() {
    // ① 本地 WS server：accept 后发一帧 hello，再主动结束连接。
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (hello_sent_tx, hello_sent_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("ws accept");
        ws.send(Message::Binary(HELLO.to_vec()))
            .await
            .expect("server send hello");
        hello_sent_tx.send(()).expect("notify hello sent");
        let probe = ws
            .next()
            .await
            .expect("client probe should arrive")
            .expect("client probe should be valid ws frame");
        match probe {
            Message::Text(text) => assert_eq!(text.as_bytes(), PROBE),
            other => panic!("client probe should be Text, got {other:?}"),
        }
        ws.send(Message::Close(None))
            .await
            .expect("server send close");
    });

    // ② NativeTransport 连接（tick_tx=Some → 生产入站路径）
    let (tick_tx, mut tick_rx) = mpsc::channel::<Tick>(16);
    let url = format!("ws://{addr}/");
    let mut transport = NativeTransport::new(url, TransportId::from_raw(0), Some(tick_tx));
    let activation = transport.connect().await.expect("connect");
    hello_sent_rx
        .await
        .expect("server should have queued hello");
    assert!(
        tick_rx.try_recv().is_err(),
        "connect 已确认 reader 到达 gate；activation 前不得发布任何生命周期/入站 Tick"
    );
    let duplicate_connect = timeout(Duration::from_secs(1), transport.connect())
        .await
        .expect("duplicate connect must fail before starting network I/O");
    assert!(
        duplicate_connect.is_err(),
        "active connection must not be overwritten by a second connect"
    );

    // 真实预填表路径：sender 先进入路由表，并且 Connected 前已可发送；随后才激活 reader。
    let transport = Arc::new(transport);
    let mut transports = TransportTable::new();
    transports.insert(TransportId::from_raw(0), Arc::clone(&transport));
    transports[&TransportId::from_raw(0)]
        .send(Bytes::from_static(PROBE))
        .await
        .expect("registered sender should work before Connected");
    activation.activate().await.expect("activate");

    let connected = timeout(Duration::from_secs(3), tick_rx.recv())
        .await
        .expect("等 Connected 超时")
        .expect("tick 通道关闭");
    assert!(matches!(connected, Tick::Connected(id) if id == TransportId::from_raw(0)));
    let inbound = timeout(Duration::from_secs(3), tick_rx.recv())
        .await
        .expect("等 Inbound 超时")
        .expect("tick 通道关闭");
    assert!(matches!(inbound, Tick::Inbound(bytes) if bytes.as_bytes() == HELLO));
    let disconnected = timeout(Duration::from_secs(3), tick_rx.recv())
        .await
        .expect("等 Disconnected 超时")
        .expect("tick 通道关闭");
    assert!(matches!(disconnected, Tick::Disconnected(id) if id == TransportId::from_raw(0)));

    server.await.expect("server task");

    // 远端 Close 已由 reader 发布 Disconnected。随后 cleanup close 即使底层 Close 写失败，
    // 也必须只做资源清理，不能从另一份 wrapper 状态再发布第二次 Disconnected。
    let _remote_close_result = transport.close().await;
    transport
        .close()
        .await
        .expect("cleanup must leave close idempotent even if the first Close write failed");
    drop(transports);
    drop(transport);
    let trailing = timeout(Duration::from_secs(1), async {
        let mut ticks = Vec::new();
        while let Some(tick) = tick_rx.recv().await {
            ticks.push(tick);
        }
        ticks
    })
    .await
    .expect("all lifecycle senders should close after cleanup");
    assert!(
        trailing.is_empty(),
        "remote-close + cleanup-close 后不得再有 Tick，实际：{trailing:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_close_still_cleans_up_and_emits_one_disconnected() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let (close_seen_tx, close_seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("ws accept");
        let close = ws
            .next()
            .await
            .expect("client close should arrive")
            .expect("client close should be a valid frame");
        assert!(matches!(close, Message::Close(_)));
        close_seen_tx.send(()).ok();
    });

    // 容量 1 先被 Connected 占满，使 cleanup 稳定阻塞在 Disconnected send。
    let (tick_tx, mut tick_rx) = mpsc::channel::<Tick>(1);
    let mut transport = NativeTransport::new(
        format!("ws://{addr}/"),
        TransportId::from_raw(0),
        Some(tick_tx),
    );
    let activation = transport.connect().await.expect("connect");
    activation.activate().await.expect("activate");
    let transport = Arc::new(transport);

    let transport_for_close = Arc::clone(&transport);
    let close_caller = tokio::spawn(async move { transport_for_close.close().await });
    timeout(Duration::from_secs(3), close_seen_rx)
        .await
        .expect("server should observe Close")
        .expect("close observation channel should stay open");

    // 只取消调用方；SharedWsClient 内部 cleanup 必须继续持有 sink/reader/lifecycle。
    close_caller.abort();
    assert!(close_caller
        .await
        .expect_err("close caller should be cancelled")
        .is_cancelled());

    {
        let concurrent_close = transport.close();
        tokio::pin!(concurrent_close);
        assert!(
            matches!(
                futures_util::poll!(concurrent_close.as_mut()),
                Poll::Pending
            ),
            "concurrent close must wait on the in-progress cleanup completion"
        );

        let connected = tick_rx.recv().await.expect("Connected should be queued");
        assert!(matches!(connected, Tick::Connected(id) if id == TransportId::from_raw(0)));
        let disconnected = timeout(Duration::from_secs(3), tick_rx.recv())
            .await
            .expect("cancelled caller must not suppress Disconnected")
            .expect("tick channel should stay open through cleanup");
        assert!(matches!(disconnected, Tick::Disconnected(id) if id == TransportId::from_raw(0)));

        timeout(Duration::from_secs(3), concurrent_close.as_mut())
            .await
            .expect("concurrent close should finish with cleanup")
            .expect("Close frame write should succeed");
    }
    timeout(Duration::from_secs(1), transport.close())
        .await
        .expect("idempotent close should not block")
        .expect("idempotent close");

    // cleanup 完成后通道 EOF，证明没有 detached reader 或重复生命周期发送者。
    drop(transport);
    assert!(
        timeout(Duration::from_secs(1), tick_rx.recv())
            .await
            .expect("cleanup should release every lifecycle sender")
            .is_none(),
        "cancelled close cleanup must not publish a duplicate Disconnected"
    );
    server.await.expect("server task");
}
