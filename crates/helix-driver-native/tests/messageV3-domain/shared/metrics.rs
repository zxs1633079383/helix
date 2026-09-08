//! 真实测试 engine 的指标生命周期；导出失败不得改变业务测试结果。

use helix_driver_host::{BoundedMetricRuntime, MetricRuntimeConfig};
use std::time::Duration;

/// 显式端点才启用导出，初始化失败保留可见的 Noop 降级。
pub(super) fn start() -> BoundedMetricRuntime {
    let endpoint = std::env::var("MESSAGEV3_METRICS_ENDPOINT")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let Some(endpoint) = endpoint else {
        eprintln!("MESSAGEV3_METRICS disabled: endpoint not configured");
        return BoundedMetricRuntime::disabled();
    };
    let config = MetricRuntimeConfig {
        enabled: true,
        service_name: "helix-messagev3-test".to_string(),
        endpoint,
        deployment_environment: if super::is_local_messagev3_profile() {
            "local"
        } else {
            "pre"
        }
        .to_string(),
        platform: "native".to_string(),
        scenario: "canary".to_string(),
        channel_shape: "none".to_string(),
        profile: "baseline".to_string(),
        queue_capacity: 8192,
        batch_max: 512,
        export_interval: Duration::from_secs(1),
        ..MetricRuntimeConfig::default()
    };
    match BoundedMetricRuntime::start_otlp(config) {
        Ok(runtime) => {
            eprintln!("MESSAGEV3_METRICS enabled: service=helix-messagev3-test platform=native");
            runtime
        }
        Err(_) => {
            eprintln!("MESSAGEV3_METRICS disabled: worker initialization failed");
            BoundedMetricRuntime::disabled()
        }
    }
}

/// Engine 结束后在 blocking pool 有界关闭；TimedOut 不代表 exporter worker 已退出。
pub(super) async fn shutdown(runtime: BoundedMetricRuntime) {
    let snapshot = runtime.snapshot();
    let outcome =
        tokio::task::spawn_blocking(move || runtime.shutdown(Duration::from_secs(3))).await;
    eprintln!("MESSAGEV3_METRICS snapshot={snapshot:?} shutdown={outcome:?}");
}
