use helix_core::effect::SqlValue;
use helix_driver_host::{HostOtelConfig, HostOtelRuntime, TraceHooks};
use helix_driver_native::{
    rows_from_reply_bytes, rows_to_reply_bytes, run_engine_loop_with_transports_and_trace,
};

#[test]
fn rows_codec_reexport_stays_compatible() {
    let rows = vec![vec![
        ("id".to_string(), SqlValue::Integer(1)),
        ("name".to_string(), SqlValue::Text("alice".to_string())),
        ("score".to_string(), SqlValue::Real(9.5)),
        ("extra".to_string(), SqlValue::Null),
    ]];
    let restored = rows_from_reply_bytes(&rows_to_reply_bytes(&rows)).expect("roundtrip");
    assert!(matches!(
        restored[0]
            .iter()
            .find(|(key, _)| key == "name")
            .map(|(_, value)| value),
        Some(SqlValue::Text(value)) if value == "alice"
    ));
}

#[test]
fn native_compat_is_noop_and_explicit_entry_accepts_enabled_hooks() {
    assert!(!TraceHooks::noop().is_otel_enabled());
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        service_name: "helix-driver-native-test".to_string(),
        endpoint: "noop".to_string(),
        protocol: "noop".to_string(),
        deployment_environment: "local".to_string(),
    });
    let hooks = TraceHooks::noop().with_otel(runtime);
    assert!(hooks.is_otel_enabled());
    let _explicit_entry = run_engine_loop_with_transports_and_trace;
}
