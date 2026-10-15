# Helix Host Driver

`helix-driver-host` 为 Helix Native 与 FFI 宿主提供共享的 SQLite、HTTP、WebSocket、
timer、事件投递与执行循环适配器。

```toml
[dependencies]
helix-driver-host = "=0.1.69"
```

本项目采用 `MIT OR Apache-2.0` 双许可证。

## 有界作用域读取

`ScopedScanSpec.limit` 是调用方给出的明确硬上限；host 不再叠加 10,000 行限制。
读取先应用 scope/filter，再按最多 900 个绑定值分块，并在同一个 SQLite 读事务内
累计实际行数。每块多读一条哨兵；超过上限返回错误，不交付部分名单。空 scope
不执行 SQL，`limit + 1` 无法表示为 SQLite 有符号整数时明确拒绝。该修改不改变
存储 schema 或 IM 权限规则。真实公开查询/5000 人群/多群并发/边界回归入口：
`cargo test -p helix-driver-host --test channel_roster_reads`。

## Trace 配置与失败上报

[`HostOtelConfig`](src/otel.rs) 默认 `enabled=false`、`service_name=helix-driver-host`、
`protocol=grpc`、`deployment_environment=local`；默认 endpoint 为
`http://opentelemetry-collector.monitoring.svc.cluster.local:4317`，实际可达性由宿主环境决定。
环境值归一到 `local/test/dev/pre/prod`，其他值回落为 `local`。

宿主选择 `HostOtelRuntime::from_env` / `from_env_with_metric_sink` 时，读取
`HELIX_OTEL_ENABLED`、`HELIX_OTEL_PROTOCOL`；服务名、端点、环境依次优先读取
`HELIX_OTEL_SERVICE_NAME` / `HELIX_OTEL_ENDPOINT` / `HELIX_OTEL_DEPLOYMENT_ENVIRONMENT`，
缺失时分别回退到 `OTEL_SERVICE_NAME` / `OTEL_EXPORTER_OTLP_ENDPOINT` /
`OTEL_DEPLOYMENT_ENVIRONMENT`。两级服务名均缺失时使用调用方传入的默认服务名；
端点与环境回退到 `HostOtelConfig` 默认值。直接传入配置的构造路径不因此自动读取环境变量。

[`TraceScope::step`](src/trace/hooks.rs) 将当前 span 的合法 TraceId 显式传入 core，
再将当轮失败登记到当前 step/root span；`record_failure` 标记 Error，
[`OTLP exporter`](src/otel/exporter.rs) 输出 `exception.type`、`exception.message`
并保留省略计数。诊断文本共享 core 的有界缓冲，driver 不解析业务事件来重建失败语义。

定向测试入口：`cargo test -p helix-driver-host --test failure_trace`。
本次仅核对配置、接线和测试源码，未执行 Collector/Jaeger 或真实业务链验收。
