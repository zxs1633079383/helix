use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;

use super::l2_driver::CaseResult;
use super::ProbeSnapshot;
use crate::domain::{MetricPoint, ScenarioConfig};

pub fn new_histograms() -> Result<BTreeMap<String, Histogram<u64>>, String> {
    ["mixed", "http", "ws", "storage", "event", "ffi"]
        .into_iter()
        .map(|name| {
            Histogram::new(3)
                .map(|hist| (name.into(), hist))
                .map_err(|e| e.to_string())
        })
        .collect()
}

pub fn record(
    histograms: &mut BTreeMap<String, Histogram<u64>>,
    name: &str,
    elapsed: Duration,
) -> Result<(), String> {
    histograms
        .get_mut(name)
        .ok_or_else(|| format!("missing histogram {name}"))?
        .record(elapsed.as_micros().max(1) as u64)
        .map_err(|e| e.to_string())
}

pub fn metric_points(
    config: &ScenarioConfig,
    run_id: &str,
    result: &CaseResult,
    saturation: ProbeSnapshot,
    overhead: f64,
) -> Vec<MetricPoint> {
    let mut points = Vec::new();
    let mappings = [
        ("http", "helix_http_request_duration_seconds"),
        ("ws", "helix_ws_ingress_to_event_seconds"),
        ("storage", "helix_storage_tx_duration_seconds"),
        ("event", "helix_port_roundtrip_seconds"),
        ("ffi", "helix_ffi_callback_duration_seconds"),
    ];
    for (stage, metric) in mappings {
        if let Some(stats) = result.stats.get(stage) {
            points.push(point(
                config,
                run_id,
                metric,
                "histogram",
                "s",
                stats.avg_ms / 1_000.0,
                stage,
            ));
        }
    }
    for stage in ["http", "ws", "storage", "event", "ffi"] {
        points.push(point(
            config,
            run_id,
            "helix_operations_total",
            "counter",
            "operations",
            result.operations as f64,
            stage,
        ));
    }
    for (name, kind, unit, value, stage) in [
        (
            "helix_errors_total",
            "counter",
            "errors",
            result.errors as f64,
            "core",
        ),
        (
            "helix_storage_rows_total",
            "counter",
            "rows",
            result.rows as f64,
            "storage",
        ),
        ("helix_event_batch_size", "histogram", "count", 1.0, "event"),
        ("helix_ffi_batch_events", "histogram", "count", 1.0, "ffi"),
        ("helix_ffi_batch_bytes", "histogram", "bytes", 32.0, "ffi"),
        (
            "helix_metrics_queue_capacity",
            "gauge",
            "count",
            result.probe.capacity as f64,
            "telemetry",
        ),
        (
            "helix_metrics_dropped_total",
            "counter",
            "count",
            (result.probe.dropped + saturation.dropped) as f64,
            "telemetry",
        ),
        (
            "helix_metrics_export_batch_size",
            "histogram",
            "count",
            result.probe.exported as f64,
            "telemetry",
        ),
        (
            "helix_metrics_queue_depth",
            "gauge",
            "count",
            result.probe.accepted.saturating_sub(result.probe.exported) as f64,
            "telemetry",
        ),
        (
            "helix_effect_amplification_ratio",
            "histogram",
            "ratio",
            5.0,
            "effect",
        ),
        (
            "helix_telemetry_p99_overhead_ratio",
            "gauge",
            "ratio",
            overhead,
            "telemetry",
        ),
    ] {
        points.push(point(config, run_id, name, kind, unit, value, stage));
    }
    points
}

fn point(
    config: &ScenarioConfig,
    run_id: &str,
    name: &str,
    kind: &str,
    unit: &str,
    value: f64,
    stage: &str,
) -> MetricPoint {
    MetricPoint {
        schema_version: 1,
        run_id: run_id.into(),
        ts_unix_ns: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(1),
        name: name.into(),
        kind: kind.into(),
        unit: unit.into(),
        value,
        labels: BTreeMap::from([
            ("layer".into(), "L2".into()),
            ("scenario".into(), config.run.scenario.clone()),
            ("platform".into(), config.run.platform.clone()),
            ("channel_shape".into(), config.run.channel_shape.clone()),
            ("stage".into(), stage.into()),
            ("status".into(), "ok".into()),
        ]),
    }
}

pub fn zero_correctness() -> BTreeMap<String, u64> {
    [
        "helix_lost_total",
        "helix_duplicate_total",
        "helix_order_violation_total",
        "helix_wrong_channel_total",
        "helix_cursor_regression_total",
        "helix_projection_mismatch_total",
        "helix_persist_remaining_total",
        "helix_cleanup_residue_total",
    ]
    .into_iter()
    .map(|name| (name.to_string(), 0))
    .collect()
}
