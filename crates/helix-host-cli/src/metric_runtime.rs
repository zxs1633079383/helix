use std::sync::Arc;
use std::time::Duration;

use helix_driver_host::{AsyncMetricSink, BoundedMetricRuntime, NoopMetricSink};

use crate::config::Config;

pub struct HostMetricRuntime {
    runtime: Option<BoundedMetricRuntime>,
    shutdown_timeout: Duration,
    sink: Arc<dyn AsyncMetricSink>,
}

impl HostMetricRuntime {
    pub fn start(config: &Config) -> Self {
        let shutdown_timeout =
            Duration::from_millis(config.observability.metrics.shutdown_timeout_ms);
        let runtime = match BoundedMetricRuntime::start_otlp(
            config.observability.metrics_config(),
        ) {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                tracing::error!(%error, "host-cli 指标 worker 启动失败，降级为 Noop");
                None
            }
        };
        let sink = runtime.as_ref().map_or_else(
            || Arc::new(NoopMetricSink) as Arc<dyn AsyncMetricSink>,
            BoundedMetricRuntime::sink,
        );
        Self {
            runtime,
            shutdown_timeout,
            sink,
        }
    }

    pub fn sink(&self) -> Arc<dyn AsyncMetricSink> {
        Arc::clone(&self.sink)
    }

    pub fn shutdown(self) {
        if let Some(runtime) = self.runtime {
            let outcome = runtime.shutdown(self.shutdown_timeout);
            tracing::info!(?outcome, "host-cli 指标 worker 已关闭");
        }
    }
}
