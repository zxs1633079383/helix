use super::*;
use helix_core::effect::TransportId;

#[tokio::test]
async fn test_send_on_disconnected_returns_err() {
    let (tx, _rx) = mpsc::channel(8);
    let transport =
        NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(1), Some(tx));

    let result = transport.send(Bytes::from_static(b"hello")).await;
    assert!(matches!(result, Err(PortError::Transport(_))));
}

#[tokio::test]
async fn test_recv_on_disconnected_returns_none() {
    let (tx, _rx) = mpsc::channel(8);
    let transport =
        NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(2), Some(tx));

    let result = transport.recv().await.expect("recv should not error");
    assert!(result.is_none(), "disconnected recv should return None");
}

#[tokio::test]
async fn test_close_on_disconnected_is_ok() {
    let (tx, _rx) = mpsc::channel(8);
    let transport =
        NativeTransport::new("ws://127.0.0.1:0/never", TransportId::from_raw(3), Some(tx));
    transport.close().await.ok();
}
