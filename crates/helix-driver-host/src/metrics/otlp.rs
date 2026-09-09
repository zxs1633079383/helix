use std::time::Duration;

use opentelemetry::metrics::{Counter, Gauge, Histogram, MeterProvider as _};
use opentelemetry::trace::TraceId;
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{
    Aggregation, Instrument as SdkInstrument, PeriodicReader, SdkMeterProvider, Stream, Temporality,
};
use opentelemetry_sdk::trace::{IdGenerator, RandomIdGenerator};
use opentelemetry_sdk::Resource;

use super::{
    BatchMetricExporter, MetricEvent, MetricExportError, MetricKind, MetricRuntimeConfig,
    ALL_METRIC_IDS,
};

// 独立 provider 输出累计快照；一次导出失败后，下次成功仍保留此前观测。
const OTLP_METRIC_TEMPORALITY: Temporality = Temporality::Cumulative;

enum Instrument {
    Counter(Counter<f64>),
    Gauge(Gauge<f64>),
    Histogram(Histogram<f64>),
}

pub(crate) struct OtlpMetricExporter {
    config: MetricRuntimeConfig,
    pipeline: Option<OtlpPipeline>,
}

struct OtlpPipeline {
    provider: SdkMeterProvider,
    instruments: Vec<Instrument>,
}

impl OtlpMetricExporter {
    pub(crate) fn new(config: MetricRuntimeConfig) -> Self {
        Self {
            config,
            pipeline: None,
        }
    }

    fn pipeline(&mut self) -> Result<&mut OtlpPipeline, MetricExportError> {
        if self.pipeline.is_none() {
            self.pipeline = Some(build_pipeline(&self.config)?);
        }
        self.pipeline.as_mut().ok_or_else(|| MetricExportError {
            message: "OTLP metrics pipeline unavailable".to_string(),
        })
    }
}

impl BatchMetricExporter for OtlpMetricExporter {
    fn export_batch(&mut self, batch: &[MetricEvent]) -> Result<(), MetricExportError> {
        let pipeline = self.pipeline()?;
        for event in batch {
            let attributes: Vec<KeyValue> = event
                .labels
                .iter()
                .map(|label| KeyValue::new(label.key.as_str(), label.value))
                .collect();
            match &pipeline.instruments[event.id as usize] {
                Instrument::Counter(counter) => counter.add(event.value, &attributes),
                Instrument::Gauge(gauge) => gauge.record(event.value, &attributes),
                Instrument::Histogram(histogram) => histogram.record(event.value, &attributes),
            }
        }
        pipeline
            .provider
            .force_flush()
            .map_err(|error| MetricExportError {
                message: error.to_string(),
            })
    }

    // 已建 pipeline 时 record 已完成；累计聚合器在 flush 失败后仍保留批次。
    fn retains_failed_batch(&self) -> bool {
        self.pipeline.is_some()
    }

    // 仅重试已记录的 pipeline，禁止懒建失败后把空 flush 误计为成功。
    fn retry_flush(&mut self) -> Result<(), MetricExportError> {
        self.pipeline
            .as_mut()
            .ok_or_else(|| MetricExportError {
                message: "OTLP metrics batch was not recorded".to_string(),
            })?
            .provider
            .force_flush()
            .map_err(|error| MetricExportError {
                message: error.to_string(),
            })
    }

    fn shutdown(&mut self, timeout: Duration) -> Result<(), MetricExportError> {
        let Some(pipeline) = self.pipeline.take() else {
            return Ok(());
        };
        pipeline
            .provider
            .shutdown_with_timeout(timeout)
            .map_err(|error| MetricExportError {
                message: error.to_string(),
            })
    }
}

// 懒建一个 provider 及其固定 Resource，业务线程不参与网络导出。
fn build_pipeline(config: &MetricRuntimeConfig) -> Result<OtlpPipeline, MetricExportError> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(config.endpoint.clone())
        .with_timeout(config.export_timeout)
        // 累计快照需要唯一 writer 身份，跨实例由 PromQL 显式聚合。
        .with_temporality(OTLP_METRIC_TEMPORALITY)
        .build()
        .map_err(|error| MetricExportError {
            message: error.to_string(),
        })?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(config.export_interval)
        .build();
    let resource = build_resource(config, new_instance_id());
    let provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
        .with_view(|instrument: &SdkInstrument| {
            histogram_boundaries(instrument.name()).and_then(|boundaries| {
                Stream::builder()
                    .with_aggregation(Aggregation::ExplicitBucketHistogram {
                        boundaries,
                        record_min_max: true,
                    })
                    .build()
                    .ok()
            })
        })
        .build();
    let meter = provider.meter("helix-driver-host");
    let instruments = ALL_METRIC_IDS
        .iter()
        .map(|id| {
            let descriptor = id.descriptor();
            match descriptor.kind {
                MetricKind::Counter => Instrument::Counter(
                    meter
                        .f64_counter(descriptor.name)
                        .with_unit(descriptor.unit)
                        .build(),
                ),
                MetricKind::Gauge => Instrument::Gauge(if descriptor.unit.is_empty() {
                    meter.f64_gauge(descriptor.name).build()
                } else {
                    meter
                        .f64_gauge(descriptor.name)
                        .with_unit(descriptor.unit)
                        .build()
                }),
                MetricKind::Histogram => Instrument::Histogram(
                    meter
                        .f64_histogram(descriptor.name)
                        .with_unit(descriptor.unit)
                        .build(),
                ),
            }
        })
        .collect();
    Ok(OtlpPipeline {
        provider,
        instruments,
    })
}

// 每个 SDK provider 创建一次独立 writer 身份，避免同进程多个 FFI handle 互相覆盖。
fn new_instance_id() -> String {
    let generator = RandomIdGenerator::default();
    loop {
        let id = generator.new_trace_id();
        if id != TraceId::INVALID {
            return id.to_string();
        }
    }
}

// 实例身份只属于 Resource；显式值覆盖环境 detector，不进入业务 MetricLabels。
fn build_resource(config: &MetricRuntimeConfig, instance_id: String) -> Resource {
    Resource::builder()
        .with_service_name(config.service_name.clone())
        .with_attribute(KeyValue::new("service.instance.id", instance_id))
        .with_attributes([
            KeyValue::new(
                "deployment.environment",
                bounded_value(
                    &config.deployment_environment,
                    &["local", "test", "dev", "pre", "prod"],
                    "unknown",
                ),
            ),
            KeyValue::new(
                "platform",
                bounded_value(
                    &config.platform,
                    &["host", "native", "ffi", "web"],
                    "unknown",
                ),
            ),
            KeyValue::new(
                "scenario",
                bounded_value(
                    &config.scenario,
                    &["canary", "l1_core", "l2_mixed"],
                    "unknown",
                ),
            ),
            KeyValue::new(
                "channel_shape",
                bounded_value(
                    &config.channel_shape,
                    &["none", "hot-1", "parallel-50", "fan-in-50-to-1-client"],
                    "unknown",
                ),
            ),
            KeyValue::new(
                "profile",
                bounded_value(
                    &config.profile,
                    &["baseline", "steady", "burst", "soak", "profile_only"],
                    "unknown",
                ),
            ),
        ])
        .build()
}

/// 为查询与整轮同步提供长耗时桶，清单页数使用独立数量桶。
fn histogram_boundaries(name: &str) -> Option<Vec<f64>> {
    if matches!(
        name,
        "helix_host_query_duration_seconds"
            | "helix_recovery_duration_seconds"
            | "helix_increment_page_duration_seconds"
    ) {
        return Some(vec![
            0.001, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0,
            600.0,
        ]);
    }
    if name == "helix_recovery_inventory_pages" {
        return Some(vec![
            1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0,
        ]);
    }
    if name.ends_with("_seconds") {
        return Some(vec![
            0.000_001, 0.000_005, 0.000_01, 0.000_025, 0.000_05, 0.000_1, 0.000_25, 0.000_5, 0.001,
            0.002_5, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
        ]);
    }
    if name.ends_with("_bytes") || name.contains("bytes_per_") {
        return Some(vec![
            64.0,
            256.0,
            1_024.0,
            4_096.0,
            16_384.0,
            65_536.0,
            262_144.0,
            1_048_576.0,
            4_194_304.0,
        ]);
    }
    if name.ends_with("_ratio") {
        return Some(vec![0.0, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 4.0, 8.0, 16.0]);
    }
    if [
        "helix_effects_per_tick",
        "helix_event_batch_size",
        "helix_ffi_batch_events",
        "helix_alloc_count_per_op",
        "helix_metrics_export_batch_size",
    ]
    .contains(&name)
    {
        return Some(vec![
            0.0, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0, 1_024.0, 4_096.0,
            16_384.0,
        ]);
    }
    None
}

fn bounded_value(value: &str, allowed: &[&'static str], fallback: &'static str) -> &'static str {
    allowed
        .iter()
        .copied()
        .find(|candidate| *candidate == value)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_value, build_resource, histogram_boundaries, new_instance_id, MetricRuntimeConfig,
        Temporality, OTLP_METRIC_TEMPORALITY,
    };
    use opentelemetry::Key;

    // 同进程的独立 provider 必须有不同 writer，且资源在生命周期内保持稳定。
    #[test]
    fn provider_resources_keep_distinct_stable_writer_identity() {
        let config = MetricRuntimeConfig::default();
        let first_id = new_instance_id();
        let second_id = new_instance_id();
        assert_ne!(first_id, second_id);
        for id in [&first_id, &second_id] {
            assert_eq!(id.len(), 32);
            assert!(id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
            assert_ne!(id, "00000000000000000000000000000000");
        }
        let first = build_resource(&config, first_id.clone());
        let second = build_resource(&config, second_id.clone());
        let key = Key::from_static_str("service.instance.id");
        assert_eq!(first.get(&key).unwrap().to_string(), first_id);
        assert_eq!(second.get(&key).unwrap().to_string(), second_id);
        assert_eq!(first.get(&key).unwrap().to_string(), first_id);
    }

    // 保持累计快照，重试只重新采集而不重复记录业务 batch。
    #[test]
    fn client_metrics_export_cumulative_per_provider() {
        assert_eq!(OTLP_METRIC_TEMPORALITY, Temporality::Cumulative);
    }

    #[test]
    fn resource_dimensions_reject_unbounded_run_ids() {
        assert_eq!(
            bounded_value("l2_mixed", &["l2_mixed"], "unknown"),
            "l2_mixed"
        );
        assert_eq!(
            bounded_value("run-20260714-user-123", &["l2_mixed"], "unknown"),
            "unknown"
        );
    }

    #[test]
    fn resource_dimensions_accept_dashboard_channel_shapes() {
        assert_eq!(
            bounded_value(
                "parallel-50",
                &["none", "hot-1", "parallel-50", "fan-in-50-to-1-client"],
                "unknown"
            ),
            "parallel-50"
        );
    }

    #[test]
    fn duration_buckets_resolve_microseconds_without_falling_into_seconds() {
        let boundaries =
            histogram_boundaries("helix_core_step_duration_seconds").expect("duration buckets");
        assert!(boundaries.contains(&0.000_25));
        assert!(boundaries.contains(&0.001));
        assert!(boundaries.windows(2).all(|pair| pair[0] < pair[1]));
    }
    /// 30s hydration 和 300s 长查询不能全部落入 +Inf 而丢失耗时分布。
    #[test]
    fn host_query_buckets_cover_existing_deadlines() {
        let boundaries = histogram_boundaries("helix_host_query_duration_seconds").unwrap();
        assert!(boundaries.contains(&15.0));
        assert!(boundaries.contains(&30.0));
        assert!(boundaries.contains(&300.0));
        assert!(boundaries.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
