use super::*;
use helix_driver_host::{MetricEvent, RecordOutcome};

struct EnabledMetricSink;

impl AsyncMetricSink for EnabledMetricSink {
    fn try_record(&self, _event: MetricEvent) -> RecordOutcome {
        RecordOutcome::Accepted
    }
}

#[tokio::test]
async fn engine_config_forwards_explicit_metric_sink_to_host_deps() {
    let metrics: Arc<dyn AsyncMetricSink> = Arc::new(EnabledMetricSink);
    let storage = NativeStorage::open("sqlite::memory:")
        .await
        .expect("open sqlite")
        .with_metric_sink(Arc::clone(&metrics));
    let network = helix_driver_host::HostNetworkConfig::new("http://127.0.0.1", "ws://127.0.0.1");
    let http = NativeHttp::with_config_and_metric_sink(network.clone(), Arc::clone(&metrics))
        .expect("build http");
    let (event_sink, _rx) = NativeEventSink::new();
    let event_sink = event_sink.with_metric_sink(Arc::clone(&metrics));
    let _transport = NativeTransport::with_config(network, TransportId::from_raw(0), None)
        .expect("build transport")
        .with_metric_sink(Arc::clone(&metrics));
    let config = EngineConfig::new(storage, http, SharedFileUploader::default(), event_sink, 1)
        .with_metric_sink(Arc::clone(&metrics));

    let deps = config.into_deps(TraceHooks::noop(), None, None);
    assert!(Arc::ptr_eq(&deps.metrics, &metrics));
    assert!(deps.metrics.is_enabled());
}
