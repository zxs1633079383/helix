# Rust OpenTelemetry Metrics GitHub 复用调研

> 调研日期：2026-08-12
> 目标：盘点 Rust 生态中可复用的 Metrics / eBPF 组件，避免在 Helix 内重复实现 SDK、OTLP exporter、Prometheus exposition 或 eBPF loader。

## 一句话结论

Helix 不需要再造一套 Metrics SDK 或 OTLP exporter：直接复用官方 [`opentelemetry-rust`](https://github.com/open-telemetry/opentelemetry-rust) 的 `opentelemetry`、`opentelemetry-sdk`、[`opentelemetry-otlp`](https://github.com/open-telemetry/opentelemetry-rust/tree/main/opentelemetry-otlp)，由 Collector 统一做批处理、重试和 Prometheus 暴露。若要采集 eBPF，优先把 eBPF 做成 Linux collector/sidecar，使用 Aya 或 libbpf-rs 采集，再接现有 OTLP 管线；不要把 eBPF loader、map 轮询或网络 exporter 放进 `helix-core`。

## 候选项目矩阵

| 项目 | 主要职责 | OTLP Metrics | Prometheus/OpenMetrics | 复用判断 |
|---|---|---:|---:|---|
| [`open-telemetry/opentelemetry-rust`](https://github.com/open-telemetry/opentelemetry-rust) | 官方 Rust OTel API、SDK、exporter 集合 | 直接 | 直接（`opentelemetry-prometheus`） | **首选**；不要再写 Meter/SDK/exporter |
| [`metrics-rs/metrics`](https://github.com/metrics-rs/metrics) | Rust 指标 facade 与 recorder 生态 | 否 | 直接（`metrics-exporter-prometheus`） | 只有已有 `metrics!` 代码时才选；不要与官方 OTel 并列引入两套规范 |
| [`prometheus/client_rust`](https://github.com/prometheus/client_rust) | 原生 Prometheus/OpenMetrics client | 否 | 直接 | 适合纯 Prometheus collector；不是 OTel Metrics SDK |
| [`palindrom615/metrics-exporter-otel`](https://github.com/palindrom615/metrics-exporter-otel) | `metrics` facade → OTel 的桥 | 间接 | 否 | 仅作为存量 facade 的过渡桥；第三方 fork、发布与维护信号较弱，不作为主路径 |
| [`eunomia-bpf/eunomia-exporter`](https://github.com/eunomia-bpf/eunomia-exporter) | eBPF 自定义指标 → Prometheus/OTel 的 Rust exporter | 直接 | 直接 | 方向最贴近需求，但仓库/文档较小且文档有陈旧风险；先 POC、审源码和许可证，再决定生产采用 |
| [`aya-rs/aya`](https://github.com/aya-rs/aya) | 纯 Rust eBPF 加载、attach、BTF/CO-RE、map/ringbuf | 否 | 否 | 只做采集底座；仍需 user-space metrics adapter/exporter |
| [`libbpf/libbpf-rs`](https://github.com/libbpf/libbpf-rs) | libbpf 的 Rust wrapper 与 skeleton/CO-RE 工具 | 否 | 否 | 需要成熟 libbpf 能力时复用；接受 C/libbpf 运行时边界 |
| [`cloudflare/ebpf_exporter`](https://github.com/cloudflare/ebpf_exporter) | eBPF 自定义指标 exporter | 非 Rust；OTel 主要是 trace 接缝 | 直接 | 可参考配置和指标语义，实际是 Go sidecar，不应当当作 Rust 依赖引入 |
| [`open-telemetry/opentelemetry-collector`](https://github.com/open-telemetry/opentelemetry-collector) | 接收、批处理、重试、转发、Prometheus 暴露 | 直接接收/转发 | 可由 exporter 暴露 | 基础设施层复用；不要嵌进 Rust core |

### 不要误当成 Metrics 的项目

- [`open-telemetry/opentelemetry-ebpf-profiler`](https://github.com/open-telemetry/opentelemetry-ebpf-profiler)：eBPF profiling / Profiles 信号，不是 OTel Metrics exporter。
- [`tokio-rs/tracing`](https://github.com/tokio-rs/tracing)：日志与 trace facade，不提供 Metrics SDK。
- [`tikv/pprof-rs`](https://github.com/tikv/pprof-rs)、[`grafana/pyroscope-rs`](https://github.com/grafana/pyroscope-rs)：CPU/profile 上传链路，不替代指标链路。

## 对 Helix 的落地建议

### 已有能力，不要重复引入

- [`crates/helix-driver-host/Cargo.toml`](/System/Volumes/Data/workspace/angular/visual/helix/crates/helix-driver-host/Cargo.toml:24) 已依赖 OTel、OTel SDK 和 OTLP 0.32，并启用 `metrics` 与 `grpc-tonic`。
- [`crates/helix-driver-host/src/metrics/otlp.rs`](/System/Volumes/Data/workspace/angular/visual/helix/crates/helix-driver-host/src/metrics/otlp.rs:3) 已有 Counter/Gauge/Histogram、`MetricExporter`、`PeriodicReader` 和 flush/shutdown 生命周期。
- [`docker/otel/otel-collector.yaml`](/System/Volumes/Data/workspace/angular/visual/helix/docker/otel/otel-collector.yaml:1) 已有 OTLP gRPC/HTTP receiver，metrics pipeline 经过 `memory_limiter`、`batch`，并暴露 Prometheus exporter `:9464`。

因此以下方案不建议采用：

1. 在 `helix-core` 或 `helix-im` 再加入 `metrics`、Prometheus client、Tokio exporter 或 eBPF 依赖。
2. 让每个业务 handler 直接调用 OTLP exporter、Prometheus remote-write 或 Pushgateway。
3. 同时维护官方 OTel SDK、`metrics-rs` recorder 和自定义 Prometheus exposition 三套指标语义。

### 如果需求是“Rust + eBPF + OTel Metrics”

建议按下面的边界落地：

```text
eBPF program
  -> Aya 或 libbpf-rs（采集 map/ringbuf）
  -> 独立 Linux collector/sidecar（有界队列、标签白名单、降采样）
  -> 官方 opentelemetry-otlp
  -> OTel Collector
  -> Prometheus / 后端
```

选择顺序：

1. 需要最快验证：先评估 `eunomia-exporter`，但必须锁定 commit、跑内核版本矩阵、审许可证和丢包/背压行为。
2. 需要自己控制采集逻辑且坚持纯 Rust：用 `aya`，在 collector 内写最小 OTel adapter。
3. 需要 libbpf/CO-RE 生态和成熟 BPF skeleton：用 `libbpf-rs`，接受其 C/libbpf 边界。
4. 仅需要应用指标而不是内核指标：直接使用现有 `helix-driver-host` OTel pipeline，不引入 eBPF。

## 采用前的最小验收清单

- 锁定 Git commit/版本，并记录 Rust、Linux kernel、BTF/CO-RE 和 Collector 版本矩阵。
- 许可证、依赖树和构建方式单独审计；尤其不要把无明确许可证或实验性 exporter 直接放进产品发行包。
- 指标名称、单位、aggregation、resource attributes 和 label 白名单先定合同；禁止把 pid、连接 ID 等高基数字段直接做 label。
- 验证 eBPF map 读取失败、Collector 不可用、队列满、进程退出时 flush 的降级行为；遥测失败不能阻断 Helix 业务路径。
- 对比“无 eBPF / 开 eBPF”CPU、内存、调度延迟和丢弃计数，不能只证明 exporter 返回 HTTP 200。

## 结论（按复用优先级）

| 优先级 | 复用对象 | 结论 |
|---:|---|---|
| P0 | 官方 `opentelemetry-rust` + `opentelemetry-otlp` + Collector | Helix 主路径，直接复用 |
| P1 | Aya 或 libbpf-rs | eBPF 采集底座，放在 driver/collector 边界 |
| P1 | `eunomia-exporter` | 可做 POC/sidecar 候选，先审再生产 |
| P2 | `metrics-rs/metrics`、`client_rust`、`metrics-exporter-otel` | 仅在已有生态或纯 Prometheus 约束下使用 |
| 排除 | tracing、profiler、OTel eBPF profiler | 不属于 Metrics，不能替代指标管线 |
