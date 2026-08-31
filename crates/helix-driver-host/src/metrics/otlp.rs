use std::time::Duration;

use opentelemetry::metrics::{Counter, Gauge, Histogram, MeterProvider as _};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{
    Aggregation, Instrument as SdkInstrument, PeriodicReader, SdkMeterProvider, Stream, Temporality,
};
use opentelemetry_sdk::Resource;

use super::{
    BatchMetricExporter, MetricEvent, MetricExportError, MetricKind, MetricRuntimeConfig,
    ALL_METRIC_IDS,
};

// 客户端实例共享低基数标签；Delta 由 Collector 统一累加，禁止退回进程级累计值互相覆盖。
const OTLP_METRIC_TEMPORALITY: Temporality = Temporality::Delta;

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

    fn retry_flush(&mut self) -> Result<(), MetricExportError> {
        self.pipeline()?
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

fn build_pipeline(config: &MetricRuntimeConfig) -> Result<OtlpPipeline, MetricExportError> {
    let exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .with_endpoint(config.endpoint.clone())
        .with_timeout(config.export_timeout)
        // 多个客户端进程共享低基数标签；Delta 让 Collector 按批累加，避免累计值互相 reset/覆盖。
        .with_temporality(OTLP_METRIC_TEMPORALITY)
        .build()
        .map_err(|error| MetricExportError {
            message: error.to_string(),
        })?;
    let reader = PeriodicReader::builder(exporter)
        .with_interval(config.export_interval)
        .build();
    let resource = Resource::builder()
        .with_service_name(config.service_name.clone())
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
        .build();
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

fn histogram_boundaries(name: &str) -> Option<Vec<f64>> {
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
    use super::{bounded_value, histogram_boundaries, Temporality, OTLP_METRIC_TEMPORALITY};

    #[test]
    fn client_metrics_export_delta_for_collector_aggregation() {
        assert_eq!(OTLP_METRIC_TEMPORALITY, Temporality::Delta);
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
}
