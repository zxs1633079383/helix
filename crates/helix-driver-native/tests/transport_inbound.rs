//! A2 集成验证：真实本地 WS server 发帧 → NativeTransport 读任务 → `Tick::Inbound`。
//!
//! 证明 sync-v2 入站半边真接通（能收 hello/increment 帧），而非仅编译通过。
//! 读任务独立持有 stream 半边，把数据帧作 `Tick::Inbound` 直接喂 tick_tx，
//! 与 `Tick::Connected`/`Disconnected` 同一通道（不碰 state Mutex，不与 send 抢锁）。

use std::time::Duration;

use futures_util::SinkExt;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_tungstenite::{accept_async, tungstenite::Message};

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::Tick;
use helix_driver_native::NativeTransport;

const HELLO: &[u8] = b"{\"action\":\"hello\"}";

#[tokio::test]
async fn inbound_ws_frame_becomes_tick_inbound() {
    // ① 本地 WS server：accept 后发一帧 hello，再保持片刻不主动断
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("ws accept");
        ws.send(Message::Binary(HELLO.to_vec()))
            .await
            .expect("server send hello");
        tokio::time::sleep(Duration::from_millis(300)).await;
    });

    // ② NativeTransport 连接（tick_tx=Some → 生产入站路径）
    let (tick_tx, mut tick_rx) = mpsc::channel::<Tick>(16);
    let url = format!("ws://{addr}/");
    let mut transport = NativeTransport::new(url, TransportId::from_raw(0), Some(tick_tx));
    transport.connect().await.expect("connect");

    // ③ 第一个 Tick = Connected（connect() 产出）
    let first = timeout(Duration::from_secs(3), tick_rx.recv())
        .await
        .expect("等 Connected 超时")
        .expect("tick 通道关闭");
    assert!(
        matches!(first, Tick::Connected(_)),
        "首个 Tick 应为 Connected，实为 {first:?}"
    );

    // ④ 第二个 Tick = Inbound(hello)（读任务转发的数据帧）
    let second = timeout(Duration::from_secs(3), tick_rx.recv())
        .await
        .expect("等 Inbound 超时")
        .expect("tick 通道关闭");
    match second {
        Tick::Inbound(bytes) => assert_eq!(
            bytes.as_bytes(),
            HELLO,
            "入站帧字节应原样透传给 Tick::Inbound"
        ),
        other => panic!("第二个 Tick 应为 Inbound，实为 {other:?}"),
    }

    server.abort();
}
