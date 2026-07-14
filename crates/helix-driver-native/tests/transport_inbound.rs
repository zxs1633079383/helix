//! A3 集成验证：平台连接生命周期 → 既有 Connected/Inbound/Disconnected Tick。
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
use helix_core::Tick;
use helix_driver_native::NativeTransport;

const HELLO: &[u8] = b"{\"action\":\"hello\"}";

#[tokio::test]
async fn connection_emits_connected_inbound_and_one_reader_disconnected() {
    // ① 本地 WS server：accept 后发一帧 hello，再主动结束连接。
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut ws = accept_async(stream).await.expect("ws accept");
        ws.send(Message::Binary(HELLO.to_vec()))
            .await
            .expect("server send hello");
        ws.send(Message::Close(None))
            .await
            .expect("server send close");
    });

    // ② NativeTransport 连接（tick_tx=Some → 生产入站路径）
    let (tick_tx, mut tick_rx) = mpsc::channel::<Tick>(16);
    let url = format!("ws://{addr}/");
    let mut transport = NativeTransport::new(url, TransportId::from_raw(0), Some(tick_tx));
    transport.connect().await.expect("connect");

    // ③ 收集三类事件，但不声明当前实现并不存在的物理先后保证：SharedWsClient reader
    // 在 connect 返回前已启动，故 Connected 可能与 reader 产生的事件竞速。A5 真链路会单独
    // 收口“注册 sender → Connected → 放行 reader”的发布屏障。
    let mut connected = 0;
    let mut inbound = 0;
    let mut reader_disconnected = 0;
    let mut reader_order = Vec::new();
    for _ in 0..3 {
        let tick = timeout(Duration::from_secs(3), tick_rx.recv())
            .await
            .expect("等生命周期 Tick 超时")
            .expect("tick 通道关闭");
        match tick {
            Tick::Connected(id) => {
                assert_eq!(id, TransportId::from_raw(0));
                connected += 1;
            }
            Tick::Inbound(bytes) => {
                assert_eq!(
                    bytes.as_bytes(),
                    HELLO,
                    "入站帧字节应原样透传给 Tick::Inbound"
                );
                inbound += 1;
                reader_order.push("inbound");
            }
            Tick::Disconnected(id) => {
                assert_eq!(id, TransportId::from_raw(0));
                reader_disconnected += 1;
                reader_order.push("disconnected");
            }
            other => panic!("不应出现其他 Tick：{other:?}"),
        }
    }

    server.await.expect("server task");
    assert_eq!(connected, 1, "native connect 应产一次 Connected");
    assert_eq!(inbound, 1, "reader 应转发一次 hello");
    assert_eq!(reader_disconnected, 1, "reader 结束应产一次 Disconnected");
    assert_eq!(
        reader_order,
        ["inbound", "disconnected"],
        "reader 内部必须先转发数据帧，再报告连接结束"
    );

    assert!(
        timeout(Duration::from_millis(100), tick_rx.recv())
            .await
            .is_err(),
        "reader 自身不得重复发 Disconnected；显式 close 去重另由 A5 真链路收口"
    );
}
