# Helix Host Driver

`helix-driver-host` 为 Helix Native 与 FFI 宿主提供共享的 SQLite、HTTP、WebSocket、
timer、事件投递与执行循环适配器。

```toml
[dependencies]
helix-driver-host = "=0.1.50"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## Trace 配置与失败上报

[`HostOtelConfig`](src/otel.rs) 默认 `enabled=false`、`service_name=helix-driver-host`、
`protocol=grpc`、`deployment_environment=local`；默认 endpoint 为
`http://opentelemetry-collector.monitoring.svc.cluster.local:4317`，实际可达性由宿主环境决定。
环境值归一到 `local/test/dev/pre/prod`，其他值回落为 `local`。

[`TraceScope::step`](src/trace/hooks.rs) 将当前 span 的合法 TraceId 显式传入 core，
再将当轮失败登记到当前 step/root span；`record_failure` 标记 Error，
[`OTLP exporter`](src/otel/exporter.rs) 输出 `exception.type`、`exception.message`
并保留省略计数。诊断文本共享 core 的有界缓冲，driver 不解析业务事件来重建失败语义。

定向测试入口：`cargo test -p helix-driver-host --test failure_trace`。
本次仅核对配置、接线和测试源码，未执行 Collector/Jaeger 或真实业务链验收。
