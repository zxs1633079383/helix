use bytes::Bytes;
use helix_core::effect::HttpRequest;
use helix_core::ports::FrameSender;
use helix_core::PortError;
use helix_driver_host::{
    AsyncMetricSink, HostHeaderRegistry, HostNetworkConfig, LabelKey, MetricEvent, MetricId,
    RecordOutcome, SharedHttpClient, SharedWsClient,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct RecordingMetricSink(Arc<Mutex<Vec<MetricEvent>>>);

impl AsyncMetricSink for RecordingMetricSink {
    fn try_record(&self, event: MetricEvent) -> RecordOutcome {
        self.0.lock().unwrap().push(event);
        RecordOutcome::Accepted
    }
}

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

#[test]
fn host_network_config_keeps_platform_supplied_java_origin() {
    let config = HostNetworkConfig::new(
        "https://go.example.test/api/cses",
        "wss://go.example.test/api/v4/websocket",
    )
    .with_default_api_base_url("https://java.example.test");

    assert_eq!(config.api_base_url, "https://go.example.test/api/cses");
    assert_eq!(config.default_api_base_url, "https://java.example.test");
}

#[tokio::test]
async fn shared_ws_send_before_connect_reports_transport_error() {
    let config = HostNetworkConfig::new("https://api.example.test", "wss://ws.example.test/socket");
    let metrics = RecordingMetricSink::default();
    let recorded = Arc::clone(&metrics.0);
    let ws = SharedWsClient::new(config)
        .unwrap()
        .with_metric_sink(Arc::new(metrics));

    let result = ws.send(Bytes::from_static(b"hello")).await;

    assert!(matches!(result, Err(PortError::Transport(_))));
    let events = recorded.lock().unwrap();
    let operations: Vec<_> = events
        .iter()
        .filter(|event| event.id == MetricId::OperationsTotal)
        .collect();
    assert_eq!(operations.len(), 1, "WS 完成边界只能记一次 operation");
    assert!(operations[0]
        .labels
        .iter()
        .any(|label| label.key == LabelKey::Status && label.value == "error"));
}

/// The authority accepts a POST then drops its response; the shared HTTP port must submit once.
#[tokio::test]
async fn response_loss_does_not_retry_http_write() {
    use helix_core::ports::HttpRequester;
    use tokio::io::AsyncReadExt;
    use tokio::time::{timeout, Duration};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let read = stream.read(&mut buffer).await.unwrap();
            assert!(read > 0);
            bytes.extend_from_slice(&buffer[..read]);
            if bytes.ends_with(b"{}") {
                break;
            }
        }
        assert!(bytes.starts_with(b"POST /write "));
        drop(stream); // Simulated committed write; no response is returned.
        assert!(
            timeout(Duration::from_millis(300), listener.accept())
                .await
                .is_err(),
            "no automatic duplicate POST"
        );
    });
    let client = SharedHttpClient::new(
        HostNetworkConfig::new(format!("http://{address}"), "ws://127.0.0.1:1")
            .with_timeout(Duration::from_secs(1)),
    )
    .unwrap();
    let result = client
        .request(HttpRequest {
            method: "POST".into(),
            url: "/write".into(),
            headers: Vec::new(),
            body: Some(Bytes::from_static(b"{}")),
        })
        .await;
    assert!(
        result.is_err(),
        "response loss must remain an unknown result"
    );
    timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
}

/// Offline is a deterministic pre-I/O terminal: no TCP connection and no HTTP write replay.
#[tokio::test]
async fn offline_admission_guard_returns_terminal_without_any_remote_request() {
    use tokio::{net::TcpListener, time::{timeout, Duration}};
    use helix_core::ports::HttpRequester;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = SharedHttpClient::new(HostNetworkConfig::new(format!("http://{address}"), "ws://127.0.0.1:1")).unwrap()
        .with_request_guard(std::sync::Arc::new(|| Err(helix_core::PortError::NetworkUnavailable)));
    for method in ["GET", "POST"] {
        let result = client.request(HttpRequest { method: method.into(), url: "/guarded".into(), headers: vec![], body: None }).await;
        assert!(matches!(result, Err(helix_core::PortError::NetworkUnavailable)));
    }
    assert!(timeout(Duration::from_millis(50), listener.accept()).await.is_err());
}

/// Upload has a typed pre-I/O rejection even when a background worker reaches it after admission.
#[tokio::test]
async fn confirmed_offline_upload_never_opens_a_remote_connection() {
    use helix_core::{ports::{FileUploader, upload::FileUploadUrls}, effect::FileUploadRequest};
    let uploader=helix_driver_host::SharedFileUploader::default().with_request_guard(Arc::new(||Err(PortError::NetworkUnavailable)));
    let result=uploader.upload(FileUploadRequest { local_path:"not-opened-while-offline".into(), object_key:"object".into(), method:"PUT".into(),
        urls:FileUploadUrls::new("http://127.0.0.1:1/upload".into(),"http://127.0.0.1:1/file".into()).unwrap(), headers:vec![],content_type:None,size:None }).await;
    assert!(matches!(result,Err(PortError::NetworkUnavailable)));
}

/// A fact change after request submission cannot reclassify a lost response as not_submitted.
#[tokio::test]
async fn inflight_response_loss_keeps_unknown_outcome_without_a_second_post() {
    use tokio::{net::TcpListener,io::AsyncReadExt,time::{timeout,Duration}};
    use std::sync::atomic::{AtomicBool,Ordering};
    use helix_core::ports::HttpRequester;
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap(); let address=listener.local_addr().unwrap();
    let online=Arc::new(AtomicBool::new(true));let fact=online.clone();
    let server=tokio::spawn(async move {
        let (mut stream,_)=listener.accept().await.unwrap();let mut bytes=[0;1024]; assert!(stream.read(&mut bytes).await.unwrap()>0);
        fact.store(false,Ordering::SeqCst); drop(stream);
        assert!(timeout(Duration::from_millis(100),listener.accept()).await.is_err());
    });
    let client=SharedHttpClient::new(HostNetworkConfig::new(format!("http://{address}"),"ws://127.0.0.1:1")).unwrap()
        .with_request_guard(Arc::new(move||if online.load(Ordering::SeqCst) {Ok(())} else {Err(PortError::NetworkUnavailable)}));
    let result=client.request(HttpRequest {method:"POST".into(),url:"/write".into(),headers:vec![],body:None}).await;
    assert!(result.is_err()); assert!(!matches!(result,Err(PortError::NetworkUnavailable)));
    server.await.unwrap();
}
