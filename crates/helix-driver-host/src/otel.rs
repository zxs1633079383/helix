use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use crate::metrics::{AsyncMetricSink, NoopMetricSink};
use crate::trace::{parse_trace_id, TraceCarrier};

mod exporter;

use exporter::OtlpExporter;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostOtelConfig {
    pub enabled: bool,
    pub service_name: String,
    pub endpoint: String,
    pub protocol: String,
    /// 真实部署环境，限定在低基数环境值域。
    pub deployment_environment: String,
}

// 与 metrics/otlp.rs 的 MetricRuntimeConfig 环境值域保持一致，避免 Trace Resource 接受任意环境名。
pub(crate) const DEPLOYMENT_ENVIRONMENT_VALUES: &[&str] = &["local", "test", "dev", "pre", "prod"];

// 将Trace环境归一到固定值域，禁止任意身份值进入资源标签。
pub(crate) fn bounded_deployment_environment(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if DEPLOYMENT_ENVIRONMENT_VALUES
        .iter()
        .any(|candidate| *candidate == normalized)
    {
        normalized
    } else {
        "local".to_string()
    }
}

impl Default for HostOtelConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: "helix-driver-host".to_string(),
            endpoint: "http://opentelemetry-collector.monitoring.svc.cluster.local:4317"
                .to_string(),
            protocol: "grpc".to_string(),
            deployment_environment: "local".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceDirection {
    Inbound,
    Outbound,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostSpanSnapshot {
    pub name: String,
    pub direction: TraceDirection,
    pub trace_id: Option<String>,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub attributes: Vec<(String, String)>,
    pub exported: bool,
    /// 有界失败事件；成功span保留空Vec，不分配。
    pub failures: Vec<helix_core::diagnostics::FailureDiagnostic>,
}

#[derive(Clone, Debug)]
pub struct HostOtelRuntime {
    inner: Arc<RuntimeInner>,
}

#[derive(Debug)]
struct RuntimeInner {
    config: HostOtelConfig,
    full_debug: bool,
    debug_identity: Mutex<Option<TraceIdentity>>,
    last_span: Mutex<Option<HostSpanSnapshot>>,
    last_completed_span: Mutex<Option<HostSpanSnapshot>>,
    seq: AtomicU64,
    exporter: Option<Arc<OtlpExporter>>,
    slow_threshold: Duration,
}

/// 受控 Trace 上下文中的当前用户标识；永不进入 Metrics label 或业务载荷。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceIdentity {
    pub user_name: Option<String>,
    pub user_id: Option<String>,
    pub company_id: Option<String>,
}

impl HostOtelRuntime {
    pub fn new(config: HostOtelConfig) -> Self {
        Self::new_with_metric_sink(config, Arc::new(NoopMetricSink))
    }

    /// 构造 trace runtime 并把 exporter 自指标写入共享有界 Metrics sink。
    pub fn new_with_metric_sink(config: HostOtelConfig, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        Self::new_with_metric_sink_and_slow_threshold(
            config,
            metrics,
            Duration::from_millis(env_u64("HELIX_OTEL_SLOW_TRACE_MS", 250)),
        )
    }

    /// 构造带显式慢 Trace 阈值的 runtime，供 YAML composition root 使用。
    pub fn new_with_metric_sink_and_slow_threshold(
        config: HostOtelConfig,
        metrics: Arc<dyn AsyncMetricSink>,
        slow_threshold: Duration,
    ) -> Self {
        Self::new_with_metric_sink_and_slow_threshold_and_capture_mode(
            config,
            metrics,
            slow_threshold,
            env_full_debug(),
        )
    }

    /// 构造带显式 full_debug 开关的 runtime，供宿主配置和 focused contract 使用。
    pub fn new_with_metric_sink_and_capture_mode(
        config: HostOtelConfig,
        metrics: Arc<dyn AsyncMetricSink>,
        full_debug: bool,
    ) -> Self {
        Self::new_with_metric_sink_and_slow_threshold_and_capture_mode(
            config,
            metrics,
            Duration::from_millis(env_u64("HELIX_OTEL_SLOW_TRACE_MS", 250)),
            full_debug,
        )
    }

    /// 共享的 runtime 构造实现；敏感 capture 状态只在实例内保存。
    fn new_with_metric_sink_and_slow_threshold_and_capture_mode(
        config: HostOtelConfig,
        metrics: Arc<dyn AsyncMetricSink>,
        slow_threshold: Duration,
        full_debug: bool,
    ) -> Self {
        let config = HostOtelConfig {
            deployment_environment: bounded_deployment_environment(&config.deployment_environment),
            ..config
        };
        let exporter = if config.enabled
            && !config.endpoint.eq_ignore_ascii_case("noop")
            && !config.protocol.eq_ignore_ascii_case("noop")
        {
            OtlpExporter::new(&config, metrics).map(Arc::new)
        } else {
            None
        };

        Self {
            inner: Arc::new(RuntimeInner {
                config,
                full_debug,
                debug_identity: Mutex::new(None),
                last_span: Mutex::new(None),
                last_completed_span: Mutex::new(None),
                seq: AtomicU64::new(1),
                exporter,
                slow_threshold,
            }),
        }
    }

    pub fn from_env(default_service_name: &str) -> Self {
        Self::from_env_with_metric_sink(default_service_name, Arc::new(NoopMetricSink))
    }

    /// 从可选环境变量构造 trace runtime，并注入 exporter 自指标 sink。
    pub fn from_env_with_metric_sink(
        default_service_name: &str,
        metrics: Arc<dyn AsyncMetricSink>,
    ) -> Self {
        let defaults = HostOtelConfig::default();
        Self::new_with_metric_sink(
            HostOtelConfig {
                enabled: env_flag("HELIX_OTEL_ENABLED", defaults.enabled),
                service_name: std::env::var("HELIX_OTEL_SERVICE_NAME")
                    .or_else(|_| std::env::var("OTEL_SERVICE_NAME"))
                    .unwrap_or_else(|_| default_service_name.to_string()),
                endpoint: std::env::var("HELIX_OTEL_ENDPOINT")
                    .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
                    .unwrap_or(defaults.endpoint),
                protocol: std::env::var("HELIX_OTEL_PROTOCOL").unwrap_or(defaults.protocol),
                deployment_environment: std::env::var("HELIX_OTEL_DEPLOYMENT_ENVIRONMENT")
                    .or_else(|_| std::env::var("OTEL_DEPLOYMENT_ENVIRONMENT"))
                    .unwrap_or(defaults.deployment_environment),
            },
            metrics,
        )
    }

    pub fn span(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
    ) -> HostSpanScope {
        self.span_with_attributes(name, direction, carrier, Vec::new())
    }

    /// 创建显式 parent 的 span，并在 full_debug 模式附加受控身份属性。
    pub fn span_with_attributes(
        &self,
        name: &'static str,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
        attributes: Vec<(&'static str, String)>,
    ) -> HostSpanScope {
        self.span_with_owned_name(name.to_string(), direction, carrier, attributes)
    }

    /// 创建允许运行时生成但已由调用方约束范围的 Span 名称。
    pub(crate) fn span_with_owned_name(
        &self,
        name: String,
        direction: TraceDirection,
        carrier: Option<&TraceCarrier>,
        attributes: Vec<(&'static str, String)>,
    ) -> HostSpanScope {
        let parent_span_id = carrier
            .and_then(|value| value.traceparent.as_deref())
            .and_then(parse_parent_span_id);
        let mut span_id = self.next_span_id();
        if parent_span_id.as_deref() == Some(span_id.as_str()) {
            span_id = self.next_span_id();
        }
        let trace_id = carrier
            .and_then(|value| value.traceparent.as_deref())
            .and_then(parse_trace_id)
            .unwrap_or_else(|| self.next_trace_id());
        let mut normalized_attributes = attributes
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect::<Vec<_>>();
        if self.inner.full_debug {
            if let Some(identity) = self.inner.debug_identity.lock().clone() {
                append_identity_attributes(&mut normalized_attributes, &identity);
            }
        }
        let snapshot = HostSpanSnapshot {
            name,
            direction,
            trace_id: Some(trace_id),
            span_id,
            parent_span_id,
            attributes: normalized_attributes,
            exported: self.inner.exporter.is_some(),
            failures: Vec::new(),
        };

        if self.inner.config.enabled && self.inner.exporter.is_none() {
            *self.inner.last_span.lock() = Some(snapshot.clone());
        }

        HostSpanScope {
            runtime: self.clone(),
            snapshot: Some(snapshot),
            baggage: carrier.and_then(|value| value.baggage.clone()),
            start_time: SystemTime::now(),
        }
    }

    pub fn last_span_for_test(&self) -> Option<HostSpanSnapshot> {
        self.inner.last_span.lock().clone()
    }

    /// 仅noop测试运行时保留最后完成的span；不改变last_span的开始时语义。
    pub fn last_completed_span_for_test(&self) -> Option<HostSpanSnapshot> {
        self.inner.last_completed_span.lock().clone()
    }

    pub fn is_enabled(&self) -> bool {
        self.inner.config.enabled
    }

    /// 返回 OTLP exporter 是否已完成本地装配；该状态不替代首批网络导出成功证据。
    pub fn is_exporter_ready(&self) -> bool {
        self.inner.config.enabled && self.inner.exporter.is_some()
    }

    /// 返回是否明确开启 full_debug；默认 safe，避免敏感字段意外进入 OTLP。
    pub fn is_full_debug(&self) -> bool {
        self.inner.full_debug
    }

    /// 设置 full_debug Trace 的当前用户上下文；调用方不得传入 token/cookie 等凭据。
    pub fn set_debug_identity(
        &self,
        user_name: Option<String>,
        user_id: Option<String>,
        company_id: Option<String>,
    ) {
        *self.inner.debug_identity.lock() = Some(TraceIdentity {
            user_name: bounded_identity(user_name),
            user_id: bounded_identity(user_id),
            company_id: bounded_identity(company_id),
        });
    }

    pub fn config(&self) -> &HostOtelConfig {
        &self.inner.config
    }

    pub fn dropped_span_count(&self) -> u64 {
        self.inner
            .exporter
            .as_ref()
            .map_or(0, |exporter| exporter.dropped_span_count())
    }

    fn next_span_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id() as u64;
        let id = mix_span_seed(nanos ^ seq.rotate_left(17) ^ pid.rotate_left(33));
        format!("{:016x}", id.max(1))
    }

    fn next_trace_id(&self) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed) as u128;
        format!("{:032x}", nanos ^ seq)
    }

    fn export(
        &self,
        mut snapshot: HostSpanSnapshot,
        baggage: Option<String>,
        start_time: SystemTime,
        end_time: SystemTime,
    ) {
        let Some(exporter) = &self.inner.exporter else {
            if self.inner.config.enabled {
                *self.inner.last_completed_span.lock() = Some(snapshot);
            }
            return;
        };
        let elapsed = end_time.duration_since(start_time).unwrap_or_default();
        if elapsed >= self.inner.slow_threshold {
            snapshot
                .attributes
                .push(("helix.slow".to_string(), "true".to_string()));
            snapshot.attributes.push((
                "helix.duration_ms".to_string(),
                elapsed.as_millis().to_string(),
            ));
        }
        exporter.try_enqueue(snapshot, baggage, start_time, end_time);
    }
}

/// 将身份属性限制为稳定、短字符串，并保持空值不进入 OTLP。
fn bounded_identity(value: Option<String>) -> Option<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.chars().take(256).collect())
}

/// 把用户身份挂在每个 full_debug span 上，避免依赖全局 active carrier。
fn append_identity_attributes(attributes: &mut Vec<(String, String)>, identity: &TraceIdentity) {
    if let Some(value) = &identity.user_name {
        attributes.push(("helix.context.user_name".to_string(), value.clone()));
    }
    if let Some(value) = &identity.user_id {
        attributes.push(("helix.context.user_id".to_string(), value.clone()));
    }
    if let Some(value) = &identity.company_id {
        attributes.push(("helix.context.company_id".to_string(), value.clone()));
    }
}

#[derive(Debug)]
pub struct HostSpanScope {
    runtime: HostOtelRuntime,
    snapshot: Option<HostSpanSnapshot>,
    baggage: Option<String>,
    start_time: SystemTime,
}

impl HostSpanScope {
    /// 为终态失败写入固定错误分类，exporter 将其映射为 OTLP Error 状态。
    pub fn set_error(&mut self, error_kind: &'static str) {
        if error_kind.is_empty() {
            return;
        }
        if let Some(snapshot) = self.snapshot.as_mut() {
            if let Some((_, value)) = snapshot
                .attributes
                .iter_mut()
                .find(|(key, _)| key == "error.type")
            {
                *value = error_kind.into();
            } else {
                snapshot
                    .attributes
                    .push(("error.type".into(), error_kind.into()));
            }
        }
    }

    /// 借用当前span的TraceId，不分配或创建新trace。
    pub fn trace_id(&self) -> Option<&str> {
        self.snapshot.as_ref()?.trace_id.as_deref()
    }

    /// 写入有界exception诊断，并标记当前span为Error；网络导出仍在既有worker。
    pub fn record_failure(&mut self, failure: &helix_core::diagnostics::FailureDiagnostic) {
        self.set_error(failure.kind);
        if let Some(snapshot) = self.snapshot.as_mut() {
            if snapshot.failures.len() < 16 {
                snapshot
                    .failures
                    .push(helix_core::diagnostics::FailureDiagnostic {
                        kind: failure.kind,
                        message: failure.message.chars().take(4096).collect(),
                    });
            }
        }
    }

    /// 为同一 Trace 创建显式 child parent；调用方必须把返回值放入本地上下文。
    pub fn child_carrier(&self) -> Option<TraceCarrier> {
        let snapshot = self.snapshot.as_ref()?;
        let trace_id = snapshot.trace_id.as_ref()?;
        let traceparent = format!("00-{trace_id}-{}-01", snapshot.span_id);
        Some(TraceCarrier {
            traceparent: Some(traceparent),
            baggage: self.baggage.clone(),
            raw_json: None,
        })
    }

    pub fn trace_id_for_test(&self) -> Option<String> {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.trace_id.clone())
    }

    /// 读取测试用 Span 名称，不暴露 exporter 或业务载荷。
    pub fn name_for_test(&self) -> &str {
        self.snapshot
            .as_ref()
            .map_or("", |snapshot| snapshot.name.as_str())
    }
}

impl Drop for HostSpanScope {
    fn drop(&mut self) {
        let Some(snapshot) = self.snapshot.take() else {
            return;
        };
        self.runtime.export(
            snapshot,
            self.baggage.take(),
            self.start_time,
            SystemTime::now(),
        );
    }
}

fn mix_span_seed(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn parse_parent_span_id(traceparent: &str) -> Option<String> {
    if parse_trace_id(traceparent).is_none() {
        return None;
    }
    Some(traceparent[36..52].to_string())
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim(),
                "1" | "true" | "TRUE" | "True" | "yes" | "YES" | "on" | "ON"
            )
        })
        .unwrap_or(default)
}

/// 仅由显式运行时开关启用敏感 Trace capture，未知值一律回到 safe。
fn env_full_debug() -> bool {
    std::env::var("HELIX_TRACE_CAPTURE_MODE")
        .map(|value| value.trim().eq_ignore_ascii_case("full_debug"))
        .unwrap_or(false)
}

/// 读取正整数环境覆盖；缺失或非法时保持可选默认值。
fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod otel_tests;
