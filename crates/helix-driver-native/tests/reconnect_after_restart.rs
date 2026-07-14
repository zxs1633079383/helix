use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::{HostNetworkConfig, SharedFileUploader};
use helix_driver_native::{
    run_engine_loop_with_transports, EngineConfig, NativeEventSink, NativeHttp, NativeStorage,
    NativeTransport, TransportTable,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backend_drop_reconnects_with_cookie_and_restores_ping_pong() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let address = listener.local_addr().expect("local address");
    let (cookie_tx, mut cookie_rx) = mpsc::unbounded_channel();
    let (ping_tx, mut ping_rx) = mpsc::unbounded_channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(run_backend(listener, cookie_tx, ping_tx, release_rx));

    let (tick_tx, tick_rx) = mpsc::channel::<Tick>(64);
    let transport_id = TransportId::from_raw(0);
    let ws_url = format!("ws://{address}/api/cses/websocket");
    let mut transport = NativeTransport::new(&ws_url, transport_id, Some(tick_tx.clone()))
        .with_handshake_headers(vec![("cookieId".to_string(), "user-444".to_string())]);
    transport.connect().await.expect("initial connect");

    let mut transports = TransportTable::new();
    transports.insert(transport_id, Arc::new(transport));
    let db_path =
        std::env::temp_dir().join(format!("helix-native-reconnect-{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db_path);
    let storage = NativeStorage::open(&format!("sqlite:{}?mode=rwc", db_path.display()))
        .await
        .expect("open storage");
    let http = NativeHttp::with_config(HostNetworkConfig::new("http://127.0.0.1:1", &ws_url))
        .expect("build http");
    let (event_sink, _) = NativeEventSink::new();
    let config = EngineConfig {
        storage,
        http,
        uploader: SharedFileUploader::default(),
        event_sink,
        metrics: Arc::new(helix_driver_host::NoopMetricSink),
        max_http_inflight: 1,
    };
    let mut shell = ExecutionShell::new();
    shell.register(ImModule::new(ImConfig {
        ws_url,
        api_base_url: "http://127.0.0.1:1".to_string(),
        ping_interval_ms: 40,
        ..Default::default()
    }));
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let engine = tokio::spawn(run_engine_loop_with_transports(
        shell,
        tick_rx,
        tick_tx,
        config,
        shutdown_rx,
        transports,
    ));

    let first_cookie = recv_within(&mut cookie_rx, "first cookie").await;
    let second_cookie = recv_within(&mut cookie_rx, "reconnect cookie").await;
    assert_eq!(first_cookie, (1, "user-444".to_string()));
    assert_eq!(second_cookie, (2, "user-444".to_string()));

    let first_ping = recv_within(&mut ping_rx, "ping before backend drop").await;
    let reconnect_ping = recv_within(&mut ping_rx, "ping after reconnect").await;
    let post_pong_ping = recv_within(&mut ping_rx, "next ping after pong").await;
    assert_eq!(first_ping.0, 1);
    assert_eq!(reconnect_ping.0, 2);
    assert_eq!(post_pong_ping.0, 2);
    assert!(first_ping.1 < reconnect_ping.1 && reconnect_ping.1 < post_pong_ping.1);

    shutdown_tx.send(()).expect("shutdown engine");
    engine.await.expect("engine task");
    release_tx.send(()).ok();
    server.await.expect("server task");
    let _ = std::fs::remove_file(db_path);
}

async fn run_backend(
    listener: TcpListener,
    cookie_tx: mpsc::UnboundedSender<(u8, String)>,
    ping_tx: mpsc::UnboundedSender<(u8, u64)>,
    release_rx: oneshot::Receiver<()>,
) {
    let mut release_rx = Some(release_rx);
    for connection in 1..=2 {
        let mut ws = accept_with_cookie(&listener, connection, &cookie_tx).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        ws.send(Message::Text(format!(
            r#"{{"v":1,"action":"hello","data":{{"connectionId":"conn-{connection}"}},"seq":0}}"#
        )))
        .await
        .expect("send hello");

        let first_seq = receive_ping(&mut ws).await;
        ping_tx.send((connection, first_seq)).expect("record ping");
        send_pong(&mut ws, first_seq).await;
        if connection == 1 {
            drop(ws);
            continue;
        }

        let next_seq = receive_ping(&mut ws).await;
        ping_tx
            .send((connection, next_seq))
            .expect("record next ping");
        send_pong(&mut ws, next_seq).await;
        let _ = release_rx.take().expect("release receiver").await;
    }
}

async fn accept_with_cookie(
    listener: &TcpListener,
    connection: u8,
    cookie_tx: &mpsc::UnboundedSender<(u8, String)>,
) -> WebSocketStream<TcpStream> {
    let (stream, _) = listener.accept().await.expect("accept ws");
    let cookie_tx = cookie_tx.clone();
    tokio_tungstenite::accept_hdr_async(stream, move |request: &Request, response: Response| {
        let cookie = request
            .headers()
            .get("cookieId")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("<missing>")
            .to_string();
        cookie_tx.send((connection, cookie)).expect("record cookie");
        Ok(response)
    })
    .await
    .expect("upgrade ws")
}

async fn receive_ping(ws: &mut WebSocketStream<TcpStream>) -> u64 {
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(message) = ws.next().await {
            let Message::Text(text) = message.expect("read ws message") else {
                continue;
            };
            let value: serde_json::Value = serde_json::from_str(&text).expect("ping json");
            if value.get("action").and_then(serde_json::Value::as_str) == Some("ping") {
                return value
                    .get("seq")
                    .and_then(serde_json::Value::as_u64)
                    .expect("ping seq");
            }
        }
        panic!("websocket closed before ping")
    })
    .await
    .expect("receive ping within timeout")
}

async fn send_pong(ws: &mut WebSocketStream<TcpStream>, seq: u64) {
    ws.send(Message::Text(format!(
        r#"{{"status":"OK","seq":{seq},"data":{{"gaps":[],"hashMismatch":false}}}}"#
    )))
    .await
    .expect("send pong");
}

async fn recv_within<T>(rx: &mut mpsc::UnboundedReceiver<T>, label: &str) -> T {
    tokio::time::timeout(Duration::from_secs(3), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("timeout waiting for {label}"))
        .unwrap_or_else(|| panic!("channel closed waiting for {label}"))
}
