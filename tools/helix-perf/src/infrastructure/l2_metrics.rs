#![cfg(feature = "sdk-metrics")]

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;

use super::l2_driver::sdk::CaseResult;
use super::ProbeSnapshot;
use crate::domain::{MetricPoint, ScenarioConfig};

pub fn new_histograms() -> Result<BTreeMap<String, Histogram<u64>>, String> {
    ["adapter_submit", "ws_rtt"]
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
    _overhead: f64,
) -> Vec<MetricPoint> {
    let mut points = Vec::new();
    let mappings = [
        ("ws_rtt", "helix_port_roundtrip_seconds"),
        ("http_port_roundtrip", "helix_port_roundtrip_seconds"),
        ("storage_port_roundtrip", "helix_port_roundtrip_seconds"),
        ("tick_queue_wait", "helix_tick_queue_wait_seconds"),
        ("event_dispatch", "helix_ws_ingress_to_event_seconds"),
        ("ffi_callback", "helix_ffi_callback_duration_seconds"),
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
    for (stage, operations) in [
        ("http", result.http_requests),
        ("ws", result.ws_frames),
        ("storage", result.rows),
        ("event", result.events),
        ("ffi", result.ffi_callbacks),
    ] {
        points.push(point(
            config,
            run_id,
            "helix_operations_total",
            "counter",
            "operations",
            operations as f64,
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
        (
            "helix_ffi_batch_bytes",
            "histogram",
            "bytes",
            result.ffi_bytes as f64 / result.ffi_callbacks.max(1) as f64,
            "ffi",
        ),
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
            result.probe.exported as f64 / result.probe.batches.max(1) as f64,
            "telemetry",
        ),
        (
            "helix_metrics_queue_depth",
            "gauge",
            "count",
            result.probe.depth as f64,
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
