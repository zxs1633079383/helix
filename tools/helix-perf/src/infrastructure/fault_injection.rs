#![cfg(feature = "sdk-metrics")]

use std::hint::black_box;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use helix_driver_host::metrics::{
    BoundedMetricRuntime, LabelKey, MetricEvent, MetricId, MetricLabels, MetricRuntimeConfig,
    RecordOutcome, ShutdownOutcome,
};
use serde::Serialize;

use super::evidence::new_run_id;

const BUSINESS_OPERATIONS: u64 = 2_000;
const MAX_SUBMIT_US: u64 = 50_000;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FaultEvidence {
    schema_version: u32,
    run_id: String,
    verdict: &'static str,
    business_main_thread_blocked: bool,
    cases: Vec<FaultCaseEvidence>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FaultCaseEvidence {
    fault: &'static str,
    endpoint: String,
    precondition: &'static str,
    business_operations: u64,
    accepted: u64,
    dropped: u64,
    business_checksum_ok: bool,
    max_try_record_us: u64,
    avg_try_record_us: f64,
    export_errors_total: u64,
    exporter_state: u64,
    shutdown: String,
    resources_cleaned: bool,
    pass: bool,
}

pub fn run_fault_matrix(root: &Path) -> Result<PathBuf, String> {
    let run_id = new_run_id("faults");
    let directory = root.join("perf/reports").join(&run_id);
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {error}"))?;

    let stopped = stopped_endpoint()?;
    let refused = stopped_endpoint()?;
    let timeout_server = HangingServer::start()?;
    let definitions = [
        (
            "collector_stopped",
            stopped,
            "临时 Collector listener 已启动后停止，探测确认端口关闭",
        ),
        (
            "dns_unreachable",
            "http://helix-perf.invalid:4317".to_string(),
            ".invalid 保留域名不可解析",
        ),
        (
            "connection_refused",
            refused,
            "随机本地端口已释放且探测返回连接拒绝",
        ),
        (
            "sustained_timeout",
            timeout_server.endpoint(),
            "TCP endpoint 接受连接但持续不返回 gRPC 响应",
        ),
    ];

    let mut cases = Vec::new();
    for (fault, endpoint, precondition) in definitions {
        cases.push(run_fault_case(fault, endpoint, precondition)?);
    }
    drop(timeout_server);
    let pass = cases.iter().all(|case| case.pass && case.resources_cleaned);
    let evidence = FaultEvidence {
        schema_version: 1,
        run_id,
        verdict: if pass { "PASS" } else { "FAIL" },
        business_main_thread_blocked: cases
            .iter()
            .any(|case| case.max_try_record_us > MAX_SUBMIT_US),
        cases,
    };
    let bytes = serde_json::to_vec_pretty(&evidence)
        .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {error}"))?;
    let path = directory.join("fault-injection.json");
    std::fs::write(&path, bytes).map_err(|error| format!("PERF_EVIDENCE_CORRUPT: {error}"))?;
    if !pass {
        return Err(format!(
            "PERF_TELEMETRY_UNAVAILABLE: 故障矩阵未闭合，证据={}",
            path.display()
        ));
    }
    Ok(path)
}

fn run_fault_case(
    fault: &'static str,
    endpoint: String,
    precondition: &'static str,
) -> Result<FaultCaseEvidence, String> {
    let config = MetricRuntimeConfig {
        enabled: true,
        service_name: format!("helix-perf-fault-{fault}"),
        endpoint: endpoint.clone(),
        deployment_environment: "test".to_string(),
        platform: "native".to_string(),
        scenario: "l2_mixed".to_string(),
        channel_shape: "parallel-50".to_string(),
        profile: "burst".to_string(),
        queue_capacity: 32,
        batch_max: 1,
        export_interval: Duration::from_millis(10),
        export_timeout: Duration::from_millis(150),
        retry_max: 0,
        max_backoff: Duration::from_millis(10),
        circuit_failure_threshold: 1,
        circuit_open_duration: Duration::from_secs(1),
    };
    let runtime = BoundedMetricRuntime::start_otlp(config)
        .map_err(|error| format!("PERF_TELEMETRY_UNAVAILABLE: {fault}: {error}"))?;
    let sink = runtime.sink();
    let labels = MetricLabels::one(LabelKey::Layer, "L2")
        .with(LabelKey::Stage, "telemetry")
        .with(LabelKey::Operation, "fault_business_probe");
    let mut accepted = 0_u64;
    let mut dropped = 0_u64;
    let mut max_us = 0_u64;
    let mut total_ns = 0_u128;
    let mut checksum = 0_u64;
    for operation in 1..=BUSINESS_OPERATIONS {
        let started = Instant::now();
        let outcome = sink.try_record(MetricEvent::counter(MetricId::OperationsTotal, 1.0, labels));
        let elapsed = started.elapsed();
        total_ns = total_ns.saturating_add(elapsed.as_nanos());
        max_us = max_us.max(elapsed.as_micros() as u64);
        match outcome {
            RecordOutcome::Accepted => accepted += 1,
            _ => dropped += 1,
        }
        checksum = checksum.wrapping_add(black_box(operation));
    }
    std::thread::sleep(Duration::from_millis(500));
    let snapshot = runtime.snapshot();
    let shutdown = runtime.shutdown(Duration::from_secs(1));
    let expected_checksum =
        BUSINESS_OPERATIONS.saturating_mul(BUSINESS_OPERATIONS.saturating_add(1)) / 2;
    let checksum_ok = checksum == expected_checksum;
    let shutdown_ok = matches!(shutdown, ShutdownOutcome::Drained);
    let pass = accepted + dropped == BUSINESS_OPERATIONS
        && checksum_ok
        && max_us <= MAX_SUBMIT_US
        && snapshot.export_errors_total > 0
        && shutdown_ok;
    Ok(FaultCaseEvidence {
        fault,
        endpoint,
        precondition,
        business_operations: BUSINESS_OPERATIONS,
        accepted,
        dropped,
        business_checksum_ok: checksum_ok,
        max_try_record_us: max_us,
        avg_try_record_us: total_ns as f64 / BUSINESS_OPERATIONS as f64 / 1_000.0,
        export_errors_total: snapshot.export_errors_total,
        exporter_state: snapshot.exporter_state,
        shutdown: format!("{shutdown:?}"),
        resources_cleaned: shutdown_ok,
        pass,
    })
}

fn stopped_endpoint() -> Result<String, String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: reserve fault port: {error}"))?;
    let address = listener
        .local_addr()
        .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: fault local_addr: {error}"))?;
    drop(listener);
    let probe = TcpStream::connect_timeout(&address, Duration::from_millis(50));
    if probe.is_ok() {
        return Err("PERF_FIXTURE_UNAVAILABLE: fault endpoint unexpectedly listening".into());
    }
    Ok(format!("http://{address}"))
}

struct HangingServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HangingServer {
    fn start() -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: timeout bind: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: timeout nonblocking: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: timeout local_addr: {error}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::Builder::new()
            .name("helix-perf-timeout-fixture".into())
            .spawn(move || {
                let mut held = Vec::new();
                while !worker_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => held.push(stream),
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: timeout worker: {error}"))?;
        Ok(Self {
            address,
            stop,
            worker: Some(worker),
        })
    }

    fn endpoint(&self) -> String {
        format!("http://{}", self.address)
    }
}

impl Drop for HangingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().and_then(|worker| worker.join().ok());
    }
}
