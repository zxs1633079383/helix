use super::*;

#[test]
fn default_config_keeps_otel_disabled() {
    let config = HostOtelConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.protocol, "grpc");
}

/// 动态命名入口必须保留命令 Span 名称，不能退回固定生命周期名称。
#[test]
fn owned_span_name_preserves_command_operation() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
    });

    let scope = runtime.span_with_owned_name(
        "helix.command.im_send_message".to_string(),
        TraceDirection::Internal,
        None,
        Vec::new(),
    );

    assert_eq!(scope.name_for_test(), "helix.command.im_send_message");
}

/// 错误标记幂等，终态 child 与持有中的 root 使用同一 Trace。
#[test]
fn query_scope_marks_error_and_links_terminal() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        endpoint: "noop".into(),
        protocol: "noop".into(),
        ..Default::default()
    });
    let mut root =
        runtime.span_with_attributes("helix.host.query", TraceDirection::Internal, None, vec![]);
    root.set_error("timeout");
    root.set_error("timeout");
    let child = runtime.span_with_attributes(
        "helix.host.query.terminal",
        TraceDirection::Internal,
        root.child_carrier().as_ref(),
        vec![],
    );
    assert_eq!(root.trace_id_for_test(), child.trace_id_for_test());
    let snapshot = root.snapshot.as_ref().unwrap();
    assert_eq!(
        snapshot
            .attributes
            .iter()
            .filter(|(key, _)| key == "error.type")
            .count(),
        1
    );
    assert_eq!(
        child.snapshot.as_ref().unwrap().parent_span_id.as_deref(),
        Some(snapshot.span_id.as_str())
    );
}

/// Host 查询指标有固定名称、类型和单位，不能退化为业务成功计数。
#[test]
fn host_query_metric_descriptors_are_stable() {
    use crate::metrics::{MetricId, MetricKind};
    for (id, name, kind, unit) in [
        (
            MetricId::HostQueryStartedTotal,
            "helix_host_query_started_total",
            MetricKind::Counter,
            "1",
        ),
        (
            MetricId::HostQueryTerminalTotal,
            "helix_host_query_terminal_total",
            MetricKind::Counter,
            "1",
        ),
        (
            MetricId::HostQueryDurationSeconds,
            "helix_host_query_duration_seconds",
            MetricKind::Histogram,
            "s",
        ),
    ] {
        assert_eq!(id.descriptor().name, name);
        assert_eq!(id.descriptor().kind, kind);
        assert_eq!(id.descriptor().unit, unit);
    }
}
