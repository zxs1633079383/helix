# Helix 指标契约参考

## 1. 当前链路

当前指标属于 `helix-driver-host` 的边缘能力：业务/Engine seam 产生 `MetricEvent`，通过
`AsyncMetricSink::try_record` 进入有界队列，由独立 worker 批量导出 OTLP。`helix-core` 和
`helix-im` 不直接依赖 OTel、Prometheus 或网络 runtime。

真实路径为：

```text
producer -> MetricEvent -> bounded try_record -> metrics worker
         -> OTLP gRPC :4317 -> Collector -> Prometheus exporter :9464
```

Helix Host 当前没有独立的 `/metrics` HTTP handler；Go IM 服务的 `/metrics` 不能当作 Helix
指标证据。

## 2. 指标定义表

| 字段 | 约束 |
|---|---|
| `MetricId` | `registry.rs` 中静态枚举，名称必须稳定且唯一 |
| canonical name | `helix_` 前缀、snake_case；Prometheus 最终名以 metadata 为准 |
| kind | `Counter` 记录增量，`Gauge` 记录当前值，`Histogram` 记录样本 |
| unit | 使用 `1`、`seconds`、`bytes`、`rows` 等明确基础单位 |
| availability | 有真实 producer 才能为 `Runtime`；否则保留 `Planned`/`ProfileOnly`/`HarnessOnly` |
| owner | 指向唯一 producer seam，不在多个层重复计数 |
| terminal | 业务成功只在 matching `PersistOk` 后的真实 `Emit` 终态计数 |

注册表新增一条后，`otlp.rs` 会从 `ALL_METRIC_IDS` 创建 instrument。注册本身不会产生样本。

## 3. 标签合同

从 `labels.rs` 的 `LabelKey` 白名单中选择，最多 8 个；值域必须静态、有限、可枚举。可选维度
包括 `operation`、`stage`、`status`、`status_class`、`error_kind`、`direction`、`platform`、
`deployment_environment`、`scenario`、`channel_shape` 和 `exporter_state`。

禁止把下列信息放入 Prometheus label：

- `corr_key`、`req_id`、run ID、temporary ID；
- user/channel/post/message/company ID；
- token、cookie、完整 URL、SQL 参数或业务文本。

这些信息如需诊断，应进入 Trace 或受控的 debug capture，而不是时序标签。

## 4. Producer 选择

| 指标语义 | 首选 owner |
|---|---|
| MessageV3 业务终态 | `metrics/business.rs` 的 event/terminal 分类器 |
| Tick、Effect、PortReply、队列、worker | `engine/perf_metrics.rs` 或 `dispatch.rs` |
| HTTP/WS/Storage adapter | 对应 host adapter 的稳定完成/失败 seam |
| exporter 自观测 | `metrics/worker.rs`，不要从业务 Handler 复制 |

不要把“HTTP 已接受”“WS 已写入”“socket write 成功”当作业务完成；不要在 `helix-im` 内
自己创建 OTel instrument。

## 5. 低侵入判定

通过标准：

1. 业务线程只执行 bounded `try_record`，不等待网络或 flush；
2. exporter、重试、熔断、shutdown 全在独立 metrics worker；
3. disabled/Noop 时业务结果不变化；
4. 不新增无界队列、同步 HTTP/DNS/TLS、全量 payload 序列化或每事件动态标签；
5. 新增函数和测试都能说明职责、不变量或边界副作用。

## 6. 官方基线

- OpenTelemetry Rust：<https://opentelemetry.io/docs/languages/rust/>
- OTel Metrics 概念与 instrument：<https://opentelemetry.io/docs/concepts/signals/metrics/>
- OTel Collector：<https://opentelemetry.io/docs/collector/>
- Prometheus API：<https://prometheus.io/docs/prometheus/latest/querying/api/>
