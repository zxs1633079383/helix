use std::time::Duration;

use opentelemetry::metrics::{Counter, Gauge, Histogram, MeterProvider as _};
use opentelemetry::KeyValue;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::Resource;

use super::{
    BatchMetricExporter, MetricEvent, MetricExportError, MetricKind, MetricRuntimeConfig,
    ALL_METRIC_IDS,
};

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
                config.deployment_environment.clone(),
            ),
            KeyValue::new("platform", config.platform.clone()),
            KeyValue::new("run.context", config.run_context.clone()),
        ])
        .build();
    let provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(resource)
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
                MetricKind::Gauge => Instrument::Gauge(
                    meter
                        .f64_gauge(descriptor.name)
                        .with_unit(descriptor.unit)
                        .build(),
                ),
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
