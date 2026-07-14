use crate::domain::{MetricPoint, RunArtifacts, RunSummary, ScenarioConfig};

#[cfg(not(feature = "sdk-metrics"))]
pub async fn execute_l2(
    _config: &ScenarioConfig,
    _run_id: &str,
) -> Result<(RunSummary, Vec<MetricPoint>, RunArtifacts), String> {
    Err("PERF_METRICS_PIPELINE_UNAVAILABLE: L2 必须启用 sdk-metrics".into())
}

#[cfg(feature = "sdk-metrics")]
pub(super) mod sdk {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::{MetricPoint, RunArtifacts, RunSummary, ScenarioConfig};
    use crate::domain::{SampleStats, Verdict};
    use crate::infrastructure::l2_case::run_case;
    use crate::infrastructure::l2_metrics::metric_points;
    use crate::infrastructure::metric_sink::{MetricProbe, ProbeSnapshot, TelemetryMode};

    pub(crate) struct CaseResult {
        pub stats: BTreeMap<String, SampleStats>,
        pub operations: u64,
        pub errors: u64,
        pub rows: u64,
        pub http_requests: u64,
        pub ws_frames: u64,
        pub events: u64,
        pub ffi_callbacks: u64,
        pub ffi_bytes: u64,
        pub probe: ProbeSnapshot,
        pub elapsed_seconds: f64,
        pub correctness: BTreeMap<String, u64>,
        pub hdr: Vec<u8>,
        pub trace: Vec<serde_json::Value>,
        pub cleanup_residue: u64,
        pub sdk_drained: bool,
    }

    pub async fn execute(
        config: &ScenarioConfig,
        run_id: &str,
    ) -> Result<(RunSummary, Vec<MetricPoint>, RunArtifacts), String> {
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
        let off_p99 = disabled
            .stats
            .get("mixed")
            .map(|s| s.p99_ms)
            .unwrap_or(f64::MAX);
        let on_p99 = enabled
            .stats
            .get("mixed")
            .map(|s| s.p99_ms)
            .unwrap_or(f64::MAX);
        let overhead = on_p99 / off_p99.max(0.000_001);
        let telemetry_ok = overhead <= config.thresholds.max_telemetry_p99_overhead_ratio;
        let saturation_ok = saturation.dropped >= config.thresholds.min_saturation_drops;
        let queue_wait_ok = enabled
            .stats
            .get("tick_queue_wait")
            .is_some_and(|stats| stats.p99_ms <= config.thresholds.max_queue_wait_p99_ms);
        let rated_drop_ratio = enabled.probe.dropped as f64 / enabled.probe.accepted.max(1) as f64;
        let rated_drop_ok = rated_drop_ratio <= config.thresholds.max_drop_ratio_rated;
        let correctness_ok = enabled.correctness.values().all(|value| *value == 0)
            && disabled.correctness.values().all(|value| *value == 0);
        let errors = disabled.errors + enabled.errors;
        let errors_zero = errors == 0;
        let hard_ok = telemetry_ok
            && saturation_ok
            && queue_wait_ok
            && rated_drop_ok
            && correctness_ok
            && errors_zero
            && disabled.sdk_drained
            && enabled.sdk_drained;
        let assertions = BTreeMap::from([
            ("correctness_zero".into(), correctness_ok),
            ("telemetry_overhead_within_budget".into(), telemetry_ok),
            ("queue_saturation_increases_drops".into(), saturation_ok),
            ("tick_queue_wait_within_budget".into(), queue_wait_ok),
            ("rated_metrics_drop_within_budget".into(), rated_drop_ok),
            (
                "metrics_shutdown_drained".into(),
                disabled.sdk_drained && enabled.sdk_drained,
            ),
            ("errors_zero".into(), errors_zero),
            ("host_driver_harness".into(), true),
        ]);
        let summary = RunSummary {
            schema_version: 1,
            run_id: run_id.into(),
            scenario_id: config.run.scenario.clone(),
            verdict: if hard_ok {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
            latency: enabled.stats.clone(),
            qps: BTreeMap::from([(
                "storage".into(),
                enabled.rows as f64 / enabled.elapsed_seconds,
            )]),
            rps: BTreeMap::from([
                (
                    "http".into(),
                    enabled.http_requests as f64 / enabled.elapsed_seconds,
                ),
                (
                    "ws_inbound".into(),
                    enabled.ws_frames as f64 / enabled.elapsed_seconds,
                ),
                (
                    "event".into(),
                    enabled.events as f64 / enabled.elapsed_seconds,
                ),
            ]),
            error_count: errors,
            error_rate: errors as f64 / (disabled.elapsed_seconds + enabled.elapsed_seconds),
            error_ratio: errors as f64 / (disabled.operations + enabled.operations).max(1) as f64,
            dropped: enabled.probe.dropped + saturation.dropped,
            correctness: enabled.correctness.clone(),
            assertions,
            errors: if hard_ok {
                Vec::new()
            } else {
                vec!["PERF_SLO_VIOLATION".into()]
            },
        };
        let metrics = metric_points(config, run_id, &enabled, saturation, overhead);
        let artifacts = RunArtifacts {
            hdr: enabled.hdr,
            trace_index: enabled.trace,
            sqlite: json!({"rows": enabled.rows, "wal": true, "pathRedacted": true}),
            cleanup: json!({"residue": enabled.cleanup_residue, "status": if enabled.cleanup_residue == 0 { "clean" } else { "dirty" }}),
        };
        Ok((summary, metrics, artifacts))
    }

    fn run_saturation(minimum: u64) -> ProbeSnapshot {
        let probe = MetricProbe::new(TelemetryMode::Enabled {
            capacity: 1,
            pause_worker: true,
        });
        for sequence in 0..minimum.saturating_add(100) {
            probe.try_record(sequence);
        }
        let (snapshot, _) = probe.shutdown();
        snapshot
    }
}

#[cfg(feature = "sdk-metrics")]
pub async fn execute_l2(
    config: &ScenarioConfig,
    run_id: &str,
) -> Result<(RunSummary, Vec<MetricPoint>, RunArtifacts), String> {
    sdk::execute(config, run_id).await
}
