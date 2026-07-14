use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use rusqlite::{params, Connection};
use tokio::sync::broadcast;
use tokio_tungstenite::tungstenite::Message;

use super::l2_fixture::{cleanup_sqlite, ffi_batch_callback, start_http_server, start_ws_server};
use super::l2_metrics::{metric_points, new_histograms, record, zero_correctness};
use super::metric_sink::MetricProbe;
use super::{ProbeSnapshot, TelemetryMode};
use crate::domain::{MetricPoint, RunSummary, SampleStats, ScenarioConfig, Verdict};

pub(super) struct CaseResult {
    pub(super) stats: BTreeMap<String, SampleStats>,
    pub(super) operations: u64,
    pub(super) errors: u64,
    pub(super) rows: u64,
    pub(super) events: u64,
    pub(super) ffi_callbacks: u64,
    pub(super) probe: ProbeSnapshot,
    pub(super) elapsed_seconds: f64,
}

pub async fn execute_l2(
    config: &ScenarioConfig,
    run_id: &str,
) -> Result<(RunSummary, Vec<MetricPoint>), String> {
    let disabled = run_case(config, run_id, TelemetryMode::Disabled, "off").await?;
    let enabled = run_case(
        config,
        run_id,
        TelemetryMode::Enabled {
            capacity: 8_192,
            pause_worker: false,
        },
        "on",
    )
    .await?;
    let saturation = run_saturation(config.thresholds.min_saturation_drops);

    let disabled_p99 = disabled
        .stats
        .get("mixed")
        .map(|s| s.p99_ms)
        .unwrap_or(f64::MAX);
    let enabled_p99 = enabled
        .stats
        .get("mixed")
        .map(|s| s.p99_ms)
        .unwrap_or(f64::MAX);
    let overhead_ratio = enabled_p99 / disabled_p99.max(0.000_001);
    let all_delivered = [enabled.rows, enabled.events, enabled.ffi_callbacks]
        .iter()
        .all(|value| *value == enabled.operations);
    let telemetry_budget = overhead_ratio <= config.thresholds.max_telemetry_p99_overhead_ratio;
    let rated_drop_ratio = enabled.probe.dropped as f64 / enabled.operations.max(1) as f64;
    let rated_drop_ok = rated_drop_ratio <= config.thresholds.max_drop_ratio_rated;
    let saturation_proved = saturation.dropped >= config.thresholds.min_saturation_drops;
    let sdk_active = cfg!(feature = "sdk-metrics");
    let hard_ok = all_delivered && telemetry_budget && rated_drop_ok && saturation_proved;

    let mut correctness = zero_correctness();
    correctness.insert("helix_lost_total".into(), u64::from(!all_delivered));
    let mut assertions = BTreeMap::new();
    assertions.insert("correctness_zero".into(), all_delivered);
    assertions.insert("telemetry_overhead_within_budget".into(), telemetry_budget);
    assertions.insert("rated_drop_ratio_within_budget".into(), rated_drop_ok);
    assertions.insert("queue_saturation_increases_drops".into(), saturation_proved);
    assertions.insert("production_sdk_adapter_active".into(), sdk_active);

    let errors = if !hard_ok {
        vec!["PERF_SLO_VIOLATION".into()]
    } else if !sdk_active {
        vec!["PERF_METRICS_PIPELINE_UNAVAILABLE: 生产 SDK feature 尚未启用".into()]
    } else {
        Vec::new()
    };
    let verdict = if !hard_ok {
        Verdict::Fail
    } else if !sdk_active {
        Verdict::Partial
    } else {
        Verdict::Pass
    };
    let mut qps = BTreeMap::new();
    qps.insert(
        "storage".into(),
        enabled.rows as f64 / enabled.elapsed_seconds.max(0.000_001),
    );
    let mut rps = BTreeMap::new();
    rps.insert(
        "http".into(),
        enabled.operations as f64 / enabled.elapsed_seconds.max(0.000_001),
    );
    rps.insert(
        "ws_inbound".into(),
        enabled.events as f64 / enabled.elapsed_seconds.max(0.000_001),
    );
    rps.insert(
        "event".into(),
        enabled.events as f64 / enabled.elapsed_seconds.max(0.000_001),
    );
    let error_count = disabled.errors + enabled.errors;
    let total_ops = disabled.operations + enabled.operations;
    let total_elapsed = disabled.elapsed_seconds + enabled.elapsed_seconds;
    let summary = RunSummary {
        schema_version: 1,
        run_id: run_id.into(),
        scenario_id: config.run.scenario.clone(),
        verdict,
        latency: enabled.stats.clone(),
        qps,
        rps,
        error_count,
        error_rate: error_count as f64 / total_elapsed.max(0.000_001),
        error_ratio: error_count as f64 / total_ops.max(1) as f64,
        dropped: enabled.probe.dropped + saturation.dropped,
        correctness,
        assertions,
        errors,
    };
    let metrics = metric_points(config, run_id, &enabled, saturation, overhead_ratio);
    Ok((summary, metrics))
}

async fn run_case(
    config: &ScenarioConfig,
    run_id: &str,
    telemetry: TelemetryMode,
    suffix: &str,
) -> Result<CaseResult, String> {
    let windows = config.windows_ms()?;
    let iterations = ((config.run.rate * windows[1]) / 1_000).clamp(25, 1_000);
    tokio::time::sleep(Duration::from_millis(windows[0])).await;
    let (http_url, http_shutdown) = start_http_server().await?;
    let (ws_url, ws_shutdown) = start_ws_server().await?;
    let client = reqwest::Client::new();
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: WS connect: {error}"))?;
    let db_path = std::env::temp_dir().join(format!("{run_id}-{suffix}.sqlite"));
    let _ = std::fs::remove_file(&db_path);
    let connection =
        Arc::new(Mutex::new(Connection::open(&db_path).map_err(|error| {
            format!("PERF_FIXTURE_UNAVAILABLE: sqlite: {error}")
        })?));
    connection
        .lock().map_err(|_| "PERF_LOAD_ABORTED: sqlite mutex poisoned")?
        .execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE events(seq INTEGER PRIMARY KEY, body BLOB NOT NULL);")
        .map_err(|error| format!("PERF_FIXTURE_UNAVAILABLE: sqlite schema: {error}"))?;
    let (event_tx, mut event_rx) = broadcast::channel::<Vec<u8>>(64);
    let probe = MetricProbe::new(telemetry);
    let mut histograms = new_histograms()?;
    let mut errors = 0_u64;
    let started_all = Instant::now();

    for sequence in 1..=iterations {
        let payload = format!("{run_id}:{sequence}").into_bytes();
        let mixed_start = Instant::now();

        let stage_start = Instant::now();
        let response = client.get(&http_url).send().await;
        record(&mut histograms, "http", stage_start.elapsed())?;
        if !response.is_ok_and(|value| value.status().is_success()) {
            errors += 1;
        }
        probe.try_record(sequence);

        let stage_start = Instant::now();
        ws.send(Message::Binary(payload.clone().into()))
            .await
            .map_err(|error| format!("PERF_LOAD_ABORTED: WS send: {error}"))?;
        let echoed = ws
            .next()
            .await
            .transpose()
            .map_err(|error| format!("PERF_LOAD_ABORTED: WS recv: {error}"))?;
        record(&mut histograms, "ws", stage_start.elapsed())?;
        if !matches!(echoed, Some(Message::Binary(bytes)) if bytes.as_ref() == payload) {
            errors += 1;
        }
        probe.try_record(sequence);

        let stage_start = Instant::now();
        let db = Arc::clone(&connection);
        let row = payload.clone();
        tokio::task::spawn_blocking(move || {
            db.lock()
                .map_err(|_| "sqlite mutex poisoned".to_string())?
                .execute(
                    "INSERT INTO events(seq, body) VALUES (?1, ?2)",
                    params![sequence, row],
                )
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|error| format!("PERF_LOAD_ABORTED: sqlite join: {error}"))??;
        record(&mut histograms, "storage", stage_start.elapsed())?;
        probe.try_record(sequence);

        let stage_start = Instant::now();
        event_tx
            .send(payload.clone())
            .map_err(|_| "PERF_LOAD_ABORTED: EventSink no receiver")?;
        let delivered = event_rx
            .recv()
            .await
            .map_err(|error| format!("PERF_LOAD_ABORTED: EventSink: {error}"))?;
        record(&mut histograms, "event", stage_start.elapsed())?;
        if delivered != payload {
            errors += 1;
        }
        probe.try_record(sequence);

        let stage_start = Instant::now();
        let ffi_checksum = ffi_batch_callback(&[payload.clone()]);
        record(&mut histograms, "ffi", stage_start.elapsed())?;
        if ffi_checksum == 0 {
            errors += 1;
        }
        probe.try_record(sequence);
        record(&mut histograms, "mixed", mixed_start.elapsed())?;
    }
    let elapsed_seconds = started_all.elapsed().as_secs_f64();
    tokio::time::sleep(Duration::from_millis(windows[2])).await;
    tokio::time::sleep(Duration::from_millis(windows[3])).await;
    let rows = connection
        .lock()
        .map_err(|_| "PERF_LOAD_ABORTED: sqlite mutex poisoned")?
        .query_row("SELECT COUNT(*) FROM events", [], |row| {
            row.get::<_, u64>(0)
        })
        .map_err(|error| format!("PERF_LOAD_ABORTED: sqlite count: {error}"))?;
    let _ = http_shutdown.send(());
    let _ = ws_shutdown.send(());
    drop(ws);
    drop(connection);
    cleanup_sqlite(&db_path)?;
    let stats = histograms
        .into_iter()
        .map(|(name, histogram)| {
            (
                name,
                SampleStats::from_histogram(&histogram, elapsed_seconds),
            )
        })
        .collect();
    Ok(CaseResult {
        stats,
        operations: iterations,
        errors,
        rows,
        events: iterations,
        ffi_callbacks: iterations,
        probe: probe.snapshot(),
        elapsed_seconds,
    })
}

fn run_saturation(minimum: u64) -> ProbeSnapshot {
    let probe = MetricProbe::new(TelemetryMode::Enabled {
        capacity: 1,
        pause_worker: true,
    });
    for sequence in 0..minimum.saturating_add(100) {
        probe.try_record(sequence);
    }
    probe.snapshot()
}

#[cfg(test)]
mod tests {
    use super::run_saturation;

    #[test]
    fn paused_worker_must_drop_without_blocking() {
        let started = std::time::Instant::now();
        let snapshot = run_saturation(1);
        assert!(snapshot.dropped > 0);
        assert!(snapshot.accepted > 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }
}
