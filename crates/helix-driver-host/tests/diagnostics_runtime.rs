//! 通过真实 tracing 入口与 HTTP 边界验证诊断字段，不把导出当作业务采集证明。
use helix_driver_host::diagnostics::{DiagnosticConfig, DiagnosticRuntime};
use helix_driver_host::{NoopMetricSink, ShutdownOutcome};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Barrier},
    time::{Duration, Instant},
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
        tracing::info!(target: "helix::diagnostics", event="channel_checkpoint", message_id="message-1", user_id="user-1", channel_id="channel-1", local_contiguous_seq=9007199254740993u64, from_seq=9007199254740993u64, next_seq=9007199254740994u64, server_horizon_seq=9007199254740995u64, token="secret", message_body="private");
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
    assert_eq!(record["message_id"], "message-1");
    assert_eq!(record["from_seq"], "9007199254740993");
    assert_eq!(record["next_seq"], "9007199254740994");
    assert_eq!(record["server_horizon_seq"], "9007199254740995");
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

/// 无有效业务记录时也输出实例级缺口，避免非法身份造成静默缺样。
#[test]
fn invalid_only_input_exports_unscoped_coverage_gap() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let body: serde_json::Value = serde_json::from_slice(&receive_body(&mut socket)).unwrap();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        body
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
    let identity = "x".repeat(129);
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id=identity.as_str());
    });
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    let body = receiver.join().unwrap();
    let record: serde_json::Value =
        serde_json::from_str(body["streams"][0]["values"][0][1].as_str().unwrap()).unwrap();
    assert_eq!(record["event"], "diagnostic_coverage_gap");
    assert_eq!(record["coverage_scope"], "producer_runtime");
    assert_eq!(record["unknown"], 1);
    assert_eq!(record["evidence_complete"], false);
    assert!(record.get("user_id").is_none());
    assert_eq!(runtime.snapshot().accepted, 0);
    assert_eq!(runtime.snapshot().exported, 1);
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
    // dropped还包含后台HTTP最终失败，不能把accepted+dropped当作producer尝试的精确分解。
    assert!(snapshot.accepted <= 1000);
    assert!(snapshot.accepted + snapshot.dropped >= 1000);
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

/// 并发producer与worker竞态下，深度不得下溢或在关闭后残留。
#[test]
fn concurrent_producers_keep_queue_depth_bounded_and_drain() {
    const PRODUCERS: usize = 8;
    const EVENTS_PER_PRODUCER: usize = 512;
    const QUEUE_CAPACITY: usize = 32;
    const BATCH_MAX: usize = 8;
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: "http://127.0.0.1:9/loki/api/v1/push".into(),
            queue_capacity: QUEUE_CAPACITY,
            batch_max: BATCH_MAX,
            export_interval: Duration::from_millis(10),
            retry_max: 0,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    let start = Arc::new(Barrier::new(PRODUCERS + 1));
    let layer = runtime.layer();
    let producers = (0..PRODUCERS)
        .map(|_| {
            let start = Arc::clone(&start);
            let layer = layer.clone();
            std::thread::spawn(move || {
                let subscriber = tracing_subscriber::registry().with(layer);
                start.wait();
                tracing::subscriber::with_default(subscriber, || {
                    for _ in 0..EVENTS_PER_PRODUCER {
                        tracing::info!(
                            target:"helix::diagnostics",
                            event="delivery_stage",
                            user_id="concurrent-user"
                        );
                    }
                });
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    let mut max_depth = 0;
    while producers.iter().any(|producer| !producer.is_finished()) {
        max_depth = max_depth.max(runtime.snapshot().queue_depth);
        std::thread::yield_now();
    }
    for producer in producers {
        producer.join().unwrap();
    }
    max_depth = max_depth.max(runtime.snapshot().queue_depth);
    assert!(
        max_depth <= QUEUE_CAPACITY + BATCH_MAX,
        "queue depth escaped queue plus one in-flight batch bound: {max_depth}"
    );
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.queue_depth, 0);
    assert_eq!(snapshot.invalid, 0);
    assert!(snapshot.accepted + snapshot.dropped >= (PRODUCERS * EVENTS_PER_PRODUCER) as u64);
}

/// 已有至少两批时，批间不应被每次额外的export_interval阻塞。
#[test]
fn full_backlog_batches_without_per_batch_interval_wait() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let mut received_at = Vec::new();
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let _ = receive_body(&mut socket);
            received_at.push(Instant::now());
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
        received_at
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            batch_max: 2,
            export_interval: Duration::from_secs(1),
            retry_max: 0,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        for _ in 0..4 {
            tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id="backlog-user");
        }
    });
    let received_at = receiver.join().unwrap();
    let elapsed = received_at[1].duration_since(received_at[0]);
    assert!(
        elapsed < Duration::from_millis(900),
        "two full batches were separated by the per-batch interval: {elapsed:?}"
    );
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    assert_eq!(runtime.snapshot().exported, 4);
}

/// 稀疏事件仍应在一次coalescing截止内导出，不能等到凑满批量。
#[test]
fn sparse_event_flushes_by_coalescing_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let _ = receive_body(&mut socket);
        let received_at = Instant::now();
        socket
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        received_at
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            batch_max: 64,
            export_interval: Duration::from_millis(100),
            retry_max: 0,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    let emitted_at = Instant::now();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id="sparse-user");
    });
    let received_at = receiver.join().unwrap();
    assert!(
        received_at.duration_since(emitted_at) < Duration::from_millis(700),
        "sparse record exceeded its coalescing flush bound: {:?}",
        received_at.duration_since(emitted_at)
    );
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    assert_eq!(runtime.snapshot().exported, 1);
}

/// 关闭信号到达后仍有限排空已接收队列，不丢弃排队记录。
#[test]
fn shutdown_drains_accepted_queue() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let _ = receive_body(&mut socket);
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            batch_max: 2,
            export_interval: Duration::from_secs(1),
            retry_max: 0,
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        for _ in 0..4 {
            tracing::info!(target:"helix::diagnostics",event="delivery_stage",user_id="shutdown-user");
        }
    });
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    receiver.join().unwrap();
    assert_eq!(runtime.snapshot().accepted, 4);
    assert_eq!(runtime.snapshot().exported, 4);
    assert_eq!(runtime.snapshot().queue_depth, 0);
}

/// 两个真实采集阶段经HTTP导出后形成同一窗口，分批上限与W3C关联保持一致。
#[test]
fn closes_received_commit_window_and_inherits_trace() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let receiver = std::thread::spawn(move || {
        let mut records = Vec::new();
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            let body: serde_json::Value =
                serde_json::from_slice(&receive_body(&mut socket)).unwrap();
            let values = body["streams"][0]["values"].as_array().unwrap();
            assert_eq!(values.len(), 1, "derived records obey batch_max too");
            for value in values {
                records.push(
                    serde_json::from_str::<serde_json::Value>(value[1].as_str().unwrap()).unwrap(),
                );
            }
            socket
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        }
        records
    });
    let mut runtime = DiagnosticRuntime::start(
        DiagnosticConfig {
            enabled: true,
            loki_url: format!("http://{address}/loki/api/v1/push"),
            batch_max: 1,
            export_interval: Duration::from_millis(10),
            ..Default::default()
        },
        Arc::new(NoopMetricSink),
    )
    .unwrap();
    tracing::subscriber::with_default(tracing_subscriber::registry().with(runtime.layer()), || {
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",stage="ws_received",result="received",tenant_id="t",user_id="u",device_session_id="d",login_attempt_id="l",channel_id="c",seq_domain="stream_seq",event_seq=9007199254740993u64,traceparent="00-0123456789abcdef0123456789abcdef-0123456789abcdef-01");
        tracing::info!(target:"helix::diagnostics",event="delivery_stage",stage="persist_terminal",result="success",tenant_id="t",user_id="u",device_session_id="d",login_attempt_id="l",channel_id="c",seq_domain="stream_seq",event_seq=9007199254740993u64);
    });
    assert_eq!(
        runtime.shutdown(Duration::from_secs(5)),
        ShutdownOutcome::Drained
    );
    let records = receiver.join().unwrap();
    assert_eq!(records[1]["trace_id"], "0123456789abcdef0123456789abcdef");
    assert_eq!(records[2]["event"], "delivery_cohort_closed");
    assert_eq!(records[2]["observed"], 1);
    assert_eq!(records[2]["committed"], 1);
    assert_eq!(records[2]["evidence_complete"], true);
    assert_eq!(runtime.snapshot().accepted, 2);
    assert_eq!(runtime.snapshot().exported, 3);
}
