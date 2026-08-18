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
