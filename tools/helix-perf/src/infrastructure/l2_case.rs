#![cfg(feature = "sdk-metrics")]

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use helix_core::effect::{ScanSpec, TransportId};
use helix_core::ports::{Storage, Transport};
use helix_core::tick::AppCommand;
use helix_core::tick::InboundBytes;
use helix_core::{ExecutionShell, Tick};
use helix_driver_host::{
    run_engine_loop_stamped, tick_ingress_channel, EngineDeps, HostNetworkConfig, HostStorage,
    MetricId, SharedFileUploader, SharedHttpClient, SharedWsClient, TraceHooks, TransportTable,
};
use helix_driver_native::{NativeClock, NativeEventSink};
use serde_json::json;
use tokio::sync::{mpsc, oneshot};

use super::ffi_probe::FfiProbe;
use super::hdr::encode_histograms;
use super::l2_driver::sdk::CaseResult;
use super::l2_event_consumer::spawn as spawn_event_consumer;
use super::l2_fixture::{
    cleanup_sqlite, payload_size, sqlite_residue, start_http_server, start_ws_server,
};
use super::l2_metrics::{new_histograms, record};
use super::l2_module::PerfModule;
use super::l2_oracle::L2Oracle;
use super::metric_sink::{MetricProbe, TelemetryMode};
use super::process_profile::{
    delta as profile_delta, export_metrics as export_profile_metrics, snapshot as profile_snapshot,
};
use crate::domain::{SampleStats, ScenarioConfig};

pub async fn run_case(
    config: &ScenarioConfig,
    run_id: &str,
    mode: TelemetryMode,
    suffix: &str,
) -> Result<CaseResult, String> {
    let windows = config.windows_ms()?;
    if config.topology.receivers != 1 {
        return Err("PERF_PROVIDER_MISSING: L2 当前真实 provider 要求 receivers=1".into());
    }
    let expected = (config.run.rate.saturating_mul(windows[1]) / 1_000).max(1);
    let profile_started = profile_snapshot();
    let saturation_mode = matches!(
        mode,
        TelemetryMode::Enabled {
            pause_worker: true,
            ..
        }
    );
    let channels = config.topology.channels;
    let oracle = Arc::new(L2Oracle::new(channels)?);
    let metric_probe = MetricProbe::new(mode);
    let metrics = metric_probe.sink();
    let (http_url, http_shutdown, http_requests) = start_http_server().await?;
    let (ws_url, ws_shutdown, ws_frames) = start_ws_server().await?;
    let db_path = std::env::temp_dir().join(format!("helix-perf-{run_id}-{suffix}.sqlite"));
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());
    let storage = Arc::new(
        HostStorage::open_sqlite_url(&db_url)
            .await
            .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: SQLite open: {e}"))?
            .with_metric_sink(Arc::clone(&metrics)),
    );
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS perf_events(
                event_key TEXT PRIMARY KEY,
                channel_no INTEGER NOT NULL,
                seq INTEGER NOT NULL,
                body BLOB NOT NULL
            );",
        )
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: SQLite schema: {e}"))?;

    let (event_sink, event_rx) = NativeEventSink::new_with_capacity(8_192);
    let event_sink = event_sink.with_metric_sink(Arc::clone(&metrics));
    let ffi_probe = Arc::new(FfiProbe::new(Arc::clone(&metrics)));
    let event_task = spawn_event_consumer(
        event_rx,
        Arc::clone(&ffi_probe),
        Arc::clone(&oracle),
        expected,
    );
    let network = HostNetworkConfig::new(http_url, ws_url).with_timeout(Duration::from_secs(3));
    let http = Arc::new(
        SharedHttpClient::new(network.clone())
            .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: HTTP client: {e}"))?
            .with_metric_sink(Arc::clone(&metrics)),
    );
    let mut ws = SharedWsClient::new(network)
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: WS client: {e}"))?
        .with_metric_sink(Arc::clone(&metrics));
    ws.connect()
        .await
        .map_err(|e| format!("PERF_FIXTURE_UNAVAILABLE: WS connect: {e}"))?;
    let ws = Arc::new(ws);

    let (tick_tx, tick_rx) = tick_ingress_channel(256);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let (_transport_tx, transport_rx) =
        mpsc::unbounded_channel::<(TransportId, Arc<SharedWsClient>)>();
    let mut shell = ExecutionShell::new();
    shell.register(PerfModule::new(Arc::clone(&oracle)));
    let deps = EngineDeps {
        storage: Arc::clone(&storage),
        http,
        uploader: Arc::new(SharedFileUploader::default()),
        event_sink: Arc::new(event_sink.clone()),
        clock: NativeClock,
        trace: TraceHooks::noop_without_otel(),
        metrics,
        max_http_inflight: 8,
        transport_lifecycle_tx: None,
        transport_trace_tx: None,
    };
    let tick_tx_for_loop = tick_tx.clone();
    let engine_task = tokio::spawn(async move {
        run_engine_loop_stamped(
            shell,
            tick_rx,
            tick_tx_for_loop,
            deps,
            shutdown_rx,
            TransportTable::new(),
            transport_rx,
        )
        .await;
    });

    tick_tx
        .send(Tick::Command(AppCommand::new(
            "perf_metric_probe",
            Bytes::from_static(b"{}"),
        )))
        .await
        .map_err(|_| "PERF_LOAD_ABORTED: command metric probe channel closed")?;

    tokio::time::sleep(Duration::from_millis(windows[0])).await;
    let mut histograms = new_histograms()?;
    let mut channel_seq = vec![0_u64; channels];
    let pacing = Duration::from_nanos(1_000_000_000_u64 / config.run.rate.max(1));
    let mut interval = tokio::time::interval(pacing);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let load_started = Instant::now();
    for operation in 1..=expected {
        interval.tick().await;
        let channel = (operation as usize - 1) % channels;
        channel_seq[channel] = channel_seq[channel].saturating_add(1);
        let seq = channel_seq[channel];
        let frame = Bytes::from(
            serde_json::to_vec(&json!({
                "operation": operation,
                "channel": channel,
                "sender": (operation as usize - 1) % config.topology.senders,
                "seq": seq,
                "payload": "x".repeat(payload_size(&config.run.payload_bucket)),
            }))
            .map_err(|e| format!("PERF_LOAD_ABORTED: encode: {e}"))?,
        );
        let operation_started = Instant::now();
        let ws_started = Instant::now();
        ws.send(frame)
            .await
            .map_err(|e| format!("PERF_LOAD_ABORTED: WS send: {e}"))?;
        let echoed = ws
            .recv()
            .await
            .map_err(|e| format!("PERF_LOAD_ABORTED: WS recv: {e}"))?
            .ok_or("PERF_LOAD_ABORTED: WS closed before echo")?;
        let ws_elapsed = ws_started.elapsed();
        record(&mut histograms, "ws_rtt", ws_elapsed)?;
        oracle.trace_hop(operation, "ws.roundtrip", ws_elapsed);
        oracle.mark_enqueued(operation);
        tick_tx
            .send(Tick::Inbound(InboundBytes(echoed)))
            .await
            .map_err(|_| "PERF_LOAD_ABORTED: engine tick channel closed")?;
        let adapter_elapsed = operation_started.elapsed();
        record(&mut histograms, "adapter_submit", adapter_elapsed)?;
        oracle.trace_hop(operation, "adapter.submit", adapter_elapsed);
    }
    let elapsed_seconds = load_started.elapsed().as_secs_f64();

    if !oracle
        .wait_complete(expected, Duration::from_secs(10))
        .await
    {
        return Err("PERF_LOAD_ABORTED: end-to-end oracle completion timeout".into());
    }
    shutdown_tx.send(()).ok();
    engine_task
        .await
        .map_err(|e| format!("PERF_LOAD_ABORTED: engine join: {e}"))?;
    tokio::time::sleep(Duration::from_millis(windows[2])).await;
    tokio::time::timeout(Duration::from_secs(10), event_task)
        .await
        .map_err(|_| "PERF_LOAD_ABORTED: event consumer drain timeout".to_string())?
        .map_err(|e| format!("PERF_LOAD_ABORTED: event consumer join: {e}"))??;
    tokio::time::sleep(Duration::from_millis(windows[3])).await;

    oracle.attach_histograms(&mut histograms);
    histograms.insert(
        "tick_queue_wait".into(),
        metric_probe.queue_wait_histogram(),
    );
    let end_to_end = histograms
        .get("end_to_end")
        .ok_or("PERF_EVIDENCE_CORRUPT: end-to-end histogram missing")?;
    if !saturation_mode {
        metric_probe.record_duration_histogram(
            MetricId::WsIngressToEventSeconds,
            end_to_end,
            "event",
        )?;
    }
    let rows = storage
        .scan(ScanSpec {
            table: "perf_events",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .map_err(|e| format!("PERF_EVIDENCE_CORRUPT: SQLite scan: {e}"))?
        .len() as u64;
    let (ffi_callbacks, ffi_events, ffi_bytes, ffi_malformed) = ffi_probe.snapshot();
    let native_events = oracle.native_events.load(Ordering::Relaxed);
    let http_count = http_requests.load(Ordering::Relaxed);
    let ws_count = ws_frames.load(Ordering::Relaxed);
    let reply_ok = oracle.port_replies_ok.load(Ordering::Relaxed);
    let completed = oracle.completed();
    let mismatch = expected.abs_diff(native_events)
        + expected.abs_diff(ffi_events)
        + expected.abs_diff(ffi_callbacks)
        + expected.abs_diff(http_count)
        + expected.abs_diff(ws_count)
        + expected.saturating_mul(2).abs_diff(reply_ok)
        + expected.abs_diff(completed);
    oracle
        .projection_mismatch
        .store(mismatch, Ordering::Relaxed);

    ws.close().await.ok();
    http_shutdown.send(()).ok();
    ws_shutdown.send(()).ok();
    drop(event_sink);
    drop(storage);
    cleanup_sqlite(&db_path)?;
    let cleanup_residue = sqlite_residue(&db_path);
    let correctness = oracle.correctness(expected, rows, cleanup_residue);
    let errors = oracle.port_replies_err.load(Ordering::Relaxed) + ffi_malformed;
    let profile = profile_delta(profile_started, expected);
    if !saturation_mode {
        let elapsed = elapsed_seconds.max(0.000_001);
        metric_probe.record_throughput(&[
            ("core", "tick", "ok", expected as f64 / elapsed),
            ("core", "tick", "error", errors as f64 / elapsed),
            ("http", "request", "ok", http_count as f64 / elapsed),
            ("ws", "inbound", "ok", ws_count as f64 / elapsed),
            ("storage", "transaction", "ok", rows as f64 / elapsed),
            ("event", "emit", "ok", native_events as f64 / elapsed),
            ("ffi", "callback", "ok", ffi_callbacks as f64 / elapsed),
        ])?;
        export_profile_metrics(metric_probe.sink().as_ref(), &profile)?;
        metric_probe.record_final(&correctness, errors)?;
    }
    let stats = histograms
        .iter()
        .map(|(name, histogram)| {
            (
                name.clone(),
                SampleStats::from_histogram(histogram, elapsed_seconds),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let hdr = encode_histograms(&histograms)?;
    let trace = oracle.trace();
    let (probe, sdk_drained) = metric_probe.shutdown();
    Ok(CaseResult {
        stats,
        operations: expected,
        errors,
        rows,
        http_requests: http_count,
        ws_frames: ws_count,
        events: native_events,
        ffi_callbacks,
        ffi_bytes,
        probe,
        elapsed_seconds,
        correctness,
        hdr,
        trace,
        cleanup_residue,
        sdk_drained,
        profile,
    })
}
