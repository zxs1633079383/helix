use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use hdrhistogram::Histogram;

use crate::domain::{MetricPoint, SampleStats, ScenarioConfig};

#[cfg(feature = "sdk-metrics")]
use helix_driver_host::metrics::{
    AsyncMetricSink, BoundedMetricRuntime, LabelKey, MetricEvent, MetricId, MetricLabels,
    MetricRuntimeConfig, RecordOutcome, ShutdownOutcome,
};
#[cfg(feature = "sdk-metrics")]
use std::time::Duration;

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

#[cfg(feature = "sdk-metrics")]
pub fn export_l1_histograms(
    config: &ScenarioConfig,
    operations: u64,
    errors: u64,
    latency: &Histogram<u64>,
    effects: &Histogram<u64>,
    effect_bytes: &Histogram<u64>,
) -> Result<(), String> {
    let runtime = BoundedMetricRuntime::start_otlp(MetricRuntimeConfig {
        enabled: true,
        service_name: "helix-perf".to_string(),
        deployment_environment: "pre".to_string(),
        platform: "native".to_string(),
        scenario: "l1_core".to_string(),
        channel_shape: bounded_channel_shape(&config.run.channel_shape).to_string(),
        profile: "steady".to_string(),
        queue_capacity: 32_768,
        batch_max: 512,
        export_interval: Duration::from_millis(10),
        ..MetricRuntimeConfig::default()
    })
    .map_err(|error| format!("PERF_TELEMETRY_UNAVAILABLE: {error}"))?;
    let sink = runtime.sink();
    let base = MetricLabels::one(LabelKey::Layer, "L1")
        .with(LabelKey::Stage, "core")
        .with(LabelKey::Operation, "step")
        .with(LabelKey::Status, "ok");
    record(
        sink.as_ref(),
        MetricEvent::counter(MetricId::OperationsTotal, operations as f64, base),
    )?;
    record(
        sink.as_ref(),
        MetricEvent::counter(MetricId::ErrorsTotal, errors as f64, base),
    )?;
    record_histogram(
        sink.as_ref(),
        MetricId::CoreStepDurationSeconds,
        latency,
        1_000_000.0,
        base,
    )?;
    record_histogram(sink.as_ref(), MetricId::EffectsPerTick, effects, 1.0, base)?;
    record_histogram(
        sink.as_ref(),
        MetricId::EffectAmplificationRatio,
        effects,
        1.0,
        base,
    )?;
    record_histogram(
        sink.as_ref(),
        MetricId::EffectBytesPerTick,
        effect_bytes,
        1.0,
        base,
    )?;
    for id in [
        MetricId::LostTotal,
        MetricId::DuplicateTotal,
        MetricId::OrderViolationTotal,
        MetricId::WrongChannelTotal,
        MetricId::CursorRegressionTotal,
        MetricId::ProjectionMismatchTotal,
        MetricId::PersistRemainingTotal,
        MetricId::CleanupResidueTotal,
    ] {
        record(sink.as_ref(), MetricEvent::counter(id, 0.0, base))?;
    }
    match runtime.shutdown(Duration::from_secs(10)) {
        ShutdownOutcome::Drained => Ok(()),
        outcome => Err(format!(
            "PERF_TELEMETRY_UNAVAILABLE: L1 聚合指标未排空: {outcome:?}"
        )),
    }
}

#[cfg(feature = "sdk-metrics")]
fn record_histogram(
    sink: &dyn AsyncMetricSink,
    id: MetricId,
    histogram: &Histogram<u64>,
    divisor: f64,
    labels: MetricLabels,
) -> Result<(), String> {
    for value in histogram.iter_recorded() {
        let sample = value.value_iterated_to() as f64 / divisor;
        for _ in 0..value.count_since_last_iteration() {
            record(sink, MetricEvent::histogram(id, sample, labels))?;
        }
    }
    Ok(())
}

#[cfg(feature = "sdk-metrics")]
fn record(sink: &dyn AsyncMetricSink, event: MetricEvent) -> Result<(), String> {
    match sink.try_record(event) {
        RecordOutcome::Accepted => Ok(()),
        outcome => Err(format!(
            "PERF_TELEMETRY_UNAVAILABLE: L1 指标入队失败: {outcome:?}"
        )),
    }
}

#[cfg(feature = "sdk-metrics")]
fn bounded_channel_shape(value: &str) -> &'static str {
    match value {
        "hot-1" => "hot-1",
        "parallel-50" => "parallel-50",
        "fan-in-50-to-1-client" => "fan-in-50-to-1-client",
        _ => "unknown",
    }
}

#[cfg(not(feature = "sdk-metrics"))]
pub fn export_l1_histograms(
    _config: &ScenarioConfig,
    _operations: u64,
    _errors: u64,
    _latency: &Histogram<u64>,
    _effects: &Histogram<u64>,
    _effect_bytes: &Histogram<u64>,
) -> Result<(), String> {
    Ok(())
}
