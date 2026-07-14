use super::*;

#[test]
fn default_config_keeps_otel_disabled() {
    let config = HostOtelConfig::default();

    assert!(!config.enabled);
    assert_eq!(config.protocol, "grpc");
}
