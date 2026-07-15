//! `config.rs` 单测（PRD §4.2）：YAML 反序列化 + 默认值 + flag 覆盖 + env 逃生口。
//! 外提 sibling（structure-gate §6：src 文件 ≤300 行，测试按关注点拆）。

use super::*;
use helix_driver_host::TraceHooks;

const FULL_YAML: &str = r#"
connection:
  ws_url:   "ws://127.0.0.1:9999/api/v4/websocket"
  api_base: "http://127.0.0.1:9999/api/cses"
  default_api_base: "http://gw.example/second"
  go_log:   "/tmp/cses-im-server.log"
  go_port:  9999
  start_hint: "irrelevant to host-cli"
identity:
  cookie_id:  "alice"
  company_id: "comp-1"
  tenant:     "tnt"
  user_name:  "Alice"
  org_name:   "Org"
  dept_name:  "Dept"
client:
  app_type:    "bct"
  device:      "ANDROID"
  device_id:   "DEV123"
  language:    "en"
  app_version: "9.9.9"
  os_version:  "android-15"
runtime:
  db:                "/tmp/x.db"
  http_max_inflight: 16
  dump_frames:       false
"#;

#[test]
fn full_yaml_deserializes_all_groups() {
    let c = Config::from_yaml_str(FULL_YAML).expect("parse ok");
    assert_eq!(c.connection.ws_url, "ws://127.0.0.1:9999/api/v4/websocket");
    assert_eq!(c.connection.api_base, "http://127.0.0.1:9999/api/cses");
    assert_eq!(c.connection.default_api_base, "http://gw.example/second");
    assert_eq!(c.identity.cookie_id, "alice");
    assert_eq!(c.identity.company_id, "comp-1");
    assert_eq!(c.identity.tenant, "tnt");
    assert_eq!(c.identity.user_name, "Alice");
    assert_eq!(c.client.device, "ANDROID");
    assert_eq!(c.client.device_id, "DEV123");
    assert_eq!(c.client.language, "en");
    assert_eq!(c.runtime.db, "/tmp/x.db");
    assert_eq!(c.runtime.http_max_inflight, 16);
    assert!(!c.runtime.dump_frames);
}

#[test]
fn script_only_unknown_fields_are_ignored() {
    // go_log/go_port/start_hint 仅脚本 yq 用；serde 默认忽略未知字段，host-cli 不建模 → 不报错。
    let c = Config::from_yaml_str(FULL_YAML).expect("unknown fields ignored");
    assert_eq!(c.connection.ws_url, "ws://127.0.0.1:9999/api/v4/websocket");
}

#[test]
fn empty_yaml_yields_defaults() {
    // 空文档 → 全分组走 Default（旧 env fallback 同值，base 8065→8066）。
    let c = Config::from_yaml_str("{}").expect("empty parse ok");
    assert_eq!(c.connection.ws_url, "ws://127.0.0.1:8066/api/v4/websocket");
    assert_eq!(c.connection.api_base, "http://127.0.0.1:8066/api/cses");
    assert_eq!(c.connection.default_api_base, "");
    assert_eq!(c.client.app_type, "bct");
    assert_eq!(c.client.device, "IOS");
    assert_eq!(c.client.language, "zh");
    assert_eq!(c.client.app_version, "1.2.56");
    assert_eq!(c.client.os_version, "macos-14.5.0");
    assert_eq!(c.identity.cookie_id, "");
    assert_eq!(c.runtime.http_max_inflight, 0);
    assert!(c.runtime.dump_frames);
}

#[test]
fn partial_group_keeps_per_field_defaults() {
    // 只给 ws_url，api_base 应走 default 工厂（非空字符串）。
    let c = Config::from_yaml_str("connection:\n  ws_url: \"ws://h:1/x\"\n").expect("partial ok");
    assert_eq!(c.connection.ws_url, "ws://h:1/x");
    assert_eq!(c.connection.api_base, "http://127.0.0.1:8066/api/cses");
}

#[test]
fn parses_observability_otel_config() {
    let c = Config::from_yaml_str(
        r#"
observability:
  otel:
    enabled: true
    serviceName: "helix-host-cli"
    endpoint: "http://127.0.0.1:4317"
    protocol: "grpc"
    sampler:
      type: "parentbased_traceidratio"
      ratio: 1.0
    facets:
      command: true
      core: true
      http: true
      ws: true
      storage: true
      portReply: true
      event: true
      native: true
      ffi: true
"#,
    )
    .expect("otel config parses");

    assert!(c.observability.otel.enabled);
    assert_eq!(c.observability.otel.service_name, "helix-host-cli");
    assert_eq!(c.observability.otel.endpoint, "http://127.0.0.1:4317");
    assert_eq!(c.observability.otel.protocol, "grpc");
    assert_eq!(
        c.observability.otel.sampler.kind,
        "parentbased_traceidratio"
    );
    assert_eq!(c.observability.otel.sampler.ratio, 1.0);
    assert!(c.observability.otel.facets.command);
    assert!(c.observability.otel.facets.core);
    assert!(c.observability.otel.facets.http);
    assert!(c.observability.otel.facets.ws);
    assert!(c.observability.otel.facets.storage);
    assert!(c.observability.otel.facets.port_reply);
    assert!(c.observability.otel.facets.event);
    assert!(c.observability.otel.facets.native);
    assert!(c.observability.otel.facets.ffi);
}

#[test]
fn default_observability_does_not_install_otel() {
    let c = Config::from_yaml_str("{}").expect("parse ok");
    let hooks = TraceHooks::noop().with_otel(c.observability.otel_runtime());
    assert!(!hooks.is_otel_enabled());
    assert!(!c.observability.metrics_config().enabled);
}

#[test]
fn parses_metrics_runtime_config_for_native_composition() {
    let c = Config::from_yaml_str(
        "observability:\n  metrics:\n    enabled: true\n    serviceName: native-test\n    endpoint: http://127.0.0.1:4317\n    deploymentEnvironment: pre\n    platform: native\n    scenario: l2_mixed\n    channelShape: parallel-50\n    profile: steady\n    queueCapacity: 4096\n    batchMax: 256\n    exportIntervalMs: 1000\n    exportTimeoutMs: 750\n    shutdownTimeoutMs: 3000\n",
    )
    .expect("metrics config parses");
    let metrics = c.observability.metrics_config();
    assert!(metrics.enabled);
    assert_eq!(metrics.service_name, "native-test");
    assert_eq!(metrics.deployment_environment, "pre");
    assert_eq!(metrics.platform, "native");
    assert_eq!(metrics.scenario, "l2_mixed");
    assert_eq!(metrics.channel_shape, "parallel-50");
    assert_eq!(metrics.queue_capacity, 4096);
    assert_eq!(metrics.batch_max, 256);
    assert_eq!(c.observability.metrics.shutdown_timeout_ms, 3000);
}

#[test]
fn yaml_enabled_observability_installs_native_hooks_without_collector_io() {
    let c = Config::from_yaml_str(
        "observability:\n  otel:\n    enabled: true\n    serviceName: native-test\n    endpoint: noop\n    protocol: noop\n",
    )
    .expect("parse ok");
    let host = c.observability.otel.host_config();
    assert_eq!(host.service_name, "native-test");
    assert_eq!(host.endpoint, "noop");
    let hooks = TraceHooks::noop().with_otel(c.observability.otel_runtime());
    assert!(hooks.is_otel_enabled());
}

#[test]
fn host_cli_env_escape_hatch_does_not_override_yaml_otel() {
    std::env::set_var("HELIX_OTEL_ENABLED", "true");
    let mut c =
        Config::from_yaml_str("observability:\n  otel:\n    enabled: false\n").expect("parse ok");
    c.apply_env_overrides();
    assert!(!c.observability.otel.enabled);
    std::env::remove_var("HELIX_OTEL_ENABLED");
}

#[test]
fn otel_endpoint_log_removes_credentials_and_query() {
    let safe = super::config_observability::endpoint_for_log(
        "https://user:secret@collector.example:4317/v1/traces?token=secret",
    );
    assert_eq!(safe, "https://collector.example:4317/v1/traces");
}

#[test]
fn malformed_yaml_errors_not_panics() {
    let r = Config::from_yaml_str("connection: [this is: not a map");
    assert!(matches!(r, Err(ConfigError::Parse(_))));
}

#[test]
fn parse_args_extracts_config_and_identity_flags() {
    let args = vec![
        "--config".to_string(),
        "/custom/helix.config.yaml".to_string(),
        "--cookie-id".to_string(),
        "999".to_string(),
        "--company-id".to_string(),
        "comp-z".to_string(),
    ];
    let cli = parse_args(args);
    assert_eq!(cli.config_path, "/custom/helix.config.yaml");
    assert_eq!(cli.cookie_id.as_deref(), Some("999"));
    assert_eq!(cli.company_id.as_deref(), Some("comp-z"));
}

#[test]
fn parse_args_defaults_config_path_when_absent() {
    let cli = parse_args(Vec::<String>::new());
    assert_eq!(cli.config_path, "./helix.config.yaml");
    assert!(cli.cookie_id.is_none());
    assert!(cli.company_id.is_none());
}

#[test]
fn cli_flag_overrides_identity() {
    let mut c = Config::from_yaml_str(FULL_YAML).expect("parse ok");
    let cli = CliArgs {
        config_path: "x".to_string(),
        cookie_id: Some("bob".to_string()),
        company_id: Some("comp-2".to_string()),
    };
    c.apply_cli_overrides(&cli);
    assert_eq!(c.identity.cookie_id, "bob");
    assert_eq!(c.identity.company_id, "comp-2");
}

#[test]
fn cli_none_leaves_identity_untouched() {
    let mut c = Config::from_yaml_str(FULL_YAML).expect("parse ok");
    let cli = CliArgs {
        config_path: "x".to_string(),
        cookie_id: None,
        company_id: None,
    };
    c.apply_cli_overrides(&cli);
    assert_eq!(c.identity.cookie_id, "alice"); // 来自 YAML，未被 None 抹掉
}

#[test]
fn env_escape_hatch_overrides_in_debug() {
    // debug 逃生口：HELIX_<FIELD> 非空覆盖 config（仅 debug；cargo test = debug_assertions on）。
    // 用 sentinel 值 + 独占字段，测后清理避免污染其它测试进程态。
    std::env::set_var("HELIX_WS_URL", "ws://sentinel:1/escape");
    std::env::set_var("HELIX_COOKIE_ID", "env-alice");
    let mut c = Config::from_yaml_str("{}").expect("parse ok");
    c.apply_env_overrides();
    assert_eq!(c.connection.ws_url, "ws://sentinel:1/escape");
    assert_eq!(c.identity.cookie_id, "env-alice");
    std::env::remove_var("HELIX_WS_URL");
    std::env::remove_var("HELIX_COOKIE_ID");
}

#[test]
fn env_escape_hatch_empty_value_does_not_clobber() {
    // 空 env 值不应抹掉 config 已有值（与旧 unwrap_or_default 行为分野：空=不覆盖）。
    std::env::set_var("HELIX_APP_VERSION", "");
    let mut c = Config::from_yaml_str("{}").expect("parse ok");
    c.apply_env_overrides();
    assert_eq!(c.client.app_version, "1.2.56"); // 仍是 default，空 env 未覆盖
    std::env::remove_var("HELIX_APP_VERSION");
}
