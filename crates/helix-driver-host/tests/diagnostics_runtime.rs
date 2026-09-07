//! 通过真实 tracing 入口与 HTTP 边界验证诊断字段，不把导出当作业务采集证明。
use helix_driver_host::diagnostics::{DiagnosticConfig, DiagnosticRuntime};
use helix_driver_host::{NoopMetricSink, ShutdownOutcome};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Arc,
    time::Duration,
};
use tracing_subscriber::prelude::*;

/// 大序号和身份留在JSON正文，普通日志与任意正文不进入诊断流。
#[test]
fn exports_exact_fields_without_sensitive_payload_or_identity_labels() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut block = [0; 4096];
        loop {
            let n = socket.read(&mut block).unwrap();
            bytes.extend_from_slice(&block[..n]);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let size: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + size {
                    let body: serde_json::Value =
                        serde_json::from_slice(&bytes[end + 4..end + 4 + size]).unwrap();
                    socket.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    return body;
                }
            }
        }
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            export_interval: Duration::from_millis(10),
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    let subscriber = tracing_subscriber::registry().with(runtime.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(user_id = "ignored", "ordinary log");
        tracing::info!(target: "helix::diagnostics", event="channel_checkpoint", user_id="user-1", channel_id="channel-1", local_contiguous_seq=9007199254740993u64, token="secret", message_body="private");
    });
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    let body = receiver.join().unwrap();
    let labels = body["streams"][0]["stream"].as_object().unwrap();
    assert_eq!(labels.len(), 3);
    assert!(!labels.contains_key("user_id"));
    let record: serde_json::Value =
        serde_json::from_str(body["streams"][0]["values"][0][1].as_str().unwrap()).unwrap();
    assert_eq!(record["local_contiguous_seq"], "9007199254740993");
    assert_eq!(record["user_id"], "user-1");
    assert!(record.get("token").is_none());
    assert!(record.get("message_body").is_none());
    assert_eq!(runtime.snapshot().exported, 1);
}

/// 超长身份整条拒绝，不截断为另一个可查询的用户。
#[test]
fn disabled_exporter_accepts_no_records() {
    let mut runtime =
        DiagnosticRuntime::start(DiagnosticConfig::default(), Arc::new(NoopMetricSink)).unwrap();
    let subscriber = tracing_subscriber::registry().with(runtime.layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target: "helix::diagnostics", event="delivery_stage", user_id="u");
    });
    assert_eq!(runtime.snapshot().accepted, 0);
    assert_eq!(
        runtime.shutdown(Duration::from_secs(1)),
        ShutdownOutcome::Disabled
    );
}

/// 超长身份拒绝整条记录，导出失败和排空均不会让它成为有效用户样本。
#[test]
fn oversized_identity_is_rejected_before_queueing() {
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: "http://127.0.0.1:9/loki/api/v1/push".into(),
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    let subscriber = tracing_subscriber::registry().with(runtime.layer());
    let identity = "x".repeat(129);
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id=identity.as_str());
    });
    assert_eq!(runtime.snapshot().invalid, 1);
    assert_eq!(runtime.snapshot().accepted, 0);
    assert_eq!(
        runtime.shutdown(Duration::from_secs(2)),
        ShutdownOutcome::Drained
    );
}

/// 仅解析本地HTTP测试请求；body保持原字节用于重试一致性验证。
fn receive_body(socket: &mut std::net::TcpStream) -> Vec<u8> {
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut block = [0; 4096];
    loop {
        let n = socket.read(&mut block).unwrap();
        assert!(n > 0, "request closed before body");
        bytes.extend_from_slice(&block[..n]);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let size: usize = String::from_utf8_lossy(&bytes[..end])
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("content-length:")
                        .map(|s| s.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() >= end + 4 + size {
                return bytes[end + 4..end + 4 + size].to_vec();
            }
        }
    }
}

/// 接收端第一次失败时保留同一event_id、业务时间和body，避免重试伪造新事件。
#[test]
fn retry_preserves_original_record_bytes() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let mut bodies = Vec::new();
        for response in ["503 Service Unavailable", "204 No Content"] {
            let (mut socket, _) = listener.accept().unwrap();
            bodies.push(receive_body(&mut socket));
            write!(
                socket,
                "HTTP/1.1 {response}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
        }
        bodies
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            export_interval: Duration::from_millis(10),
            retry_max: 1,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id="retry-user");
    });
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    let bodies = receiver.join().unwrap();
    assert_eq!(bodies[0], bodies[1]);
    assert_eq!(runtime.snapshot().errors, 1);
    assert_eq!(runtime.snapshot().exported, 1);
}

/// 网络不可用时队列不扩张；每次入队尝试都必须计为接受或明确丢弃。
#[test]
fn saturated_queue_rejects_without_unbounded_growth() {
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: "http://127.0.0.1:9/loki/api/v1/push".into(),
            queue_capacity: 1,
            batch_max: 1,
            export_interval: Duration::from_secs(10),
            retry_max: 0,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        for _ in 0..1000 {
            tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id="queue-user");
        }
    });
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.accepted + snapshot.dropped, 1000);
    assert!(snapshot.dropped > 0);
    assert!(
        snapshot.queue_depth <= 2,
        "one queued plus at most one waiting worker record"
    );
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    assert_eq!(runtime.snapshot().exported, 0);
    assert_eq!(runtime.snapshot().queue_depth, 0);
}
