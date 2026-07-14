use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;

use crate::domain::{MetricPoint, SampleStats, ScenarioConfig};

pub fn l1_metric_points(
    config: &ScenarioConfig,
    run_id: &str,
    stats: &SampleStats,
    effects: u64,
    operations: u64,
    effects_hist: &Histogram<u64>,
    bytes_hist: &Histogram<u64>,
) -> Vec<MetricPoint> {
    let labels = labels(config, "core", "step");
    vec![
        point(
            run_id,
            "helix_operations_total",
            "counter",
            "operations",
            operations as f64,
            labels.clone(),
        ),
        point(
            run_id,
            "helix_core_step_duration_seconds",
            "histogram",
            "s",
            stats.avg_ms / 1_000.0,
            labels.clone(),
        ),
        point(
            run_id,
            "helix_effects_per_tick",
            "histogram",
            "count",
            effects_hist.mean(),
            labels.clone(),
        ),
        point(
            run_id,
            "helix_effect_bytes_per_tick",
            "histogram",
            "bytes",
            bytes_hist.mean(),
            labels.clone(),
        ),
        point(
            run_id,
            "helix_effect_amplification_ratio",
            "histogram",
            "ratio",
            effects as f64 / operations.max(1) as f64,
            labels.clone(),
        ),
        // allocations/op 需要 profile provider；未测量时不写 0 冒充样本。
    ]
}

fn point(
    run_id: &str,
    name: &str,
    kind: &str,
    unit: &str,
    value: f64,
    labels: BTreeMap<String, String>,
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
        labels,
    }
}

fn labels(config: &ScenarioConfig, stage: &str, operation: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("layer".into(), "L1".into()),
        ("scenario".into(), config.run.scenario.clone()),
        ("platform".into(), config.run.platform.clone()),
        ("channel_shape".into(), config.run.channel_shape.clone()),
        ("payload_bucket".into(), config.run.payload_bucket.clone()),
        ("stage".into(), stage.into()),
        ("operation".into(), operation.into()),
        ("status".into(), "ok".into()),
    ])
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
