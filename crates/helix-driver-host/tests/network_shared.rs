use bytes::Bytes;
use helix_core::effect::HttpRequest;
use helix_core::ports::FrameSender;
use helix_core::PortError;
use helix_driver_host::{HostHeaderRegistry, HostNetworkConfig, SharedHttpClient, SharedWsClient};

#[tokio::test]
async fn header_registry_updates_and_removes_headers() {
    let registry = HostHeaderRegistry::default();

    registry
        .set_header("authorization", "Bearer first")
        .await
        .unwrap();
    registry.set_header("x-device-id", "pc-1").await.unwrap();
    registry
        .set_header("authorization", "Bearer second")
        .await
        .unwrap();
    registry.remove_header("x-device-id").await.unwrap();

    let headers = registry.snapshot().await;
    assert_eq!(headers.len(), 1);
    assert_eq!(
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer second")
    );
    assert!(headers.get("x-device-id").is_none());
}

#[tokio::test]
async fn header_registry_rejects_invalid_header_names() {
    let registry = HostHeaderRegistry::default();

    let result = registry.set_header("bad header", "value").await;

    assert!(matches!(result, Err(PortError::Http(_))));
}

#[tokio::test]
async fn shared_http_resolves_relative_urls_and_keeps_request_headers_explicit() {
    let config = HostNetworkConfig::new("https://api.example.test/root", "wss://ws.example.test");
    let registry = HostHeaderRegistry::default();
    registry
        .set_header("authorization", "Bearer global")
        .await
        .unwrap();
    registry.set_header("x-device-id", "pc-1").await.unwrap();

    let client = SharedHttpClient::with_registry(config, registry).unwrap();
    let req = HttpRequest {
        method: "POST".to_string(),
        url: "/messages".to_string(),
        headers: vec![("authorization".to_string(), "Bearer request".to_string())],
        body: Some(Bytes::from_static(b"{}")),
    };

    let prepared = client.prepare_for_test(req).await.unwrap();

    assert_eq!(prepared.method, "POST");
    assert_eq!(prepared.url, "https://api.example.test/messages");
    assert_eq!(
        prepared.headers.get("authorization").map(String::as_str),
        Some("Bearer request")
    );
    assert_eq!(
        prepared.headers.get("x-device-id").map(String::as_str),
        Some("pc-1")
    );
}

#[tokio::test]
async fn shared_ws_send_before_connect_reports_transport_error() {
    let config = HostNetworkConfig::new("https://api.example.test", "wss://ws.example.test/socket");
    let ws = SharedWsClient::new(config).unwrap();

    let result = ws.send(Bytes::from_static(b"hello")).await;

    assert!(matches!(result, Err(PortError::Transport(_))));
}
