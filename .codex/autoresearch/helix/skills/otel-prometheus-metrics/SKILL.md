---
name: otel-prometheus-metrics
description: "在 Helix 中先评审指标契约，再通过既有有界 AsyncMetricSink 低侵入编写 OTel/Prometheus 指标，并用只读脚本验证 Collector scrape、Prometheus metadata 与非空近期样本。用于新增或修改 crates/helix-driver-host/src/metrics、helix-host-cli 观测配置、docker/otel、perf/grafana，或排查指标未采集；禁止把 HTTP 200、synthetic canary 或静态注册当作业务采集成功。"
---

# Helix OTel Prometheus 指标

本 Skill 只负责指标契约、低侵入接入和可证伪采集验证。按 L0 → L1 → L2 → L3
逐级披露；没有命中当前任务的参考文件不要预加载。

## L0：先评审，后写入

每次触发都执行以下步骤：

1. 运行 `python3 ~/.codex/scripts/log_preflight.py`，检查 `git status --short --branch`，读取根
   `AGENTS.md`、`docs/GOAL.md`、`docs/ROADMAP.md` §2、`docs/harness/OPERATING.md`。
2. 对任务运行 `.agents/scripts/agent-route --summary "..." --paths "..."`，读取 Router 返回的
   required references；代码符号优先使用 codebase-memory 的 `search_graph`、`trace_path`、
   `get_code_snippet`。
3. 先生成不改生产代码的 `Metric Review`，至少写明：
   - `MetricId`、Prometheus canonical name、kind、unit；
   - owner seam、真实触发条件和 terminal/非 terminal 语义；
   - ≤8 个白名单低基数标签及每个值域；
   - 业务线程成本、队列满时行为、关闭/重试行为；
   - focused、worker/export、Collector scrape、Prometheus query、真实业务触发各自的证据命令；
   - 失败终态、回滚边界和不修改的文件。
4. 先输出 `REVIEW: PROPOSED`，交给用户或独立 reviewer。只有收到明确的
   `REVIEW: APPROVED` 才能修改指标生产代码、配置或生成看板；没有批准必须停止并返回
   `BLOCKED_REVIEW`。Agent 不得自行把自己的 proposal 视作批准。

评审必须先回答三个问题：指标是否真的可行动、是否可以在现有 seam 闭合、是否能在不阻塞
主业务线程的前提下取得真实采集证据。任一答案为否，先改 proposal，不写代码。

## L1：按需读取当前真源

根据任务选择最小集合：

- 注册与标签：`crates/helix-driver-host/src/metrics/registry.rs`、`event.rs`、`labels.rs`、
  `sink.rs`；
- 非阻塞运行时：`runtime.rs`、`worker.rs`、`otlp.rs`；
- 业务终态：`business.rs`；Engine/Effect 生命周期：`engine/perf_metrics.rs`、`dispatch.rs`；
- Host/FFI 接线：`crates/helix-host-cli/src/config_observability.rs`、`metric_runtime.rs`、
  `main.rs`、`crates/helix-driver-ffi/src/api/handle.rs`；
- 当前指标合同：`docs/Metrics/Prometheus指标说明.md`；
- Collector/Prometheus：`docker/otel/otel-collector.yaml`、`docker/otel/prometheus.yml`；
- 看板：`perf/grafana/dashboard.contract.json`、`generate-dashboard.mjs`、`contract-check.mjs`；
- 运行验证：`perf/telemetry/telemetry.sh`（只把 canary 当基础设施预检，不当业务指标证明）。

详细的映射和证据等级见：

- [指标契约参考](references/metric-contract.md)
- [运行验证与证据参考](references/runtime-verification.md)
- [只读 Prometheus 验证脚本](scripts/verify_prometheus_metric.py)

## L2：批准后的最小实现

### 1. 注册

在 `registry.rs` 的 `metric_registry!` 中增加一条 `MetricId => (name, kind, unit)`。默认
`Runtime`；只有真的没有生产 seam 时才标记 `Planned`、`ProfileOnly` 或 `HarnessOnly`。
OTLP exporter 会遍历 `ALL_METRIC_IDS` 自动建立 instrument；新的 Histogram 需要特殊桶时才
同步 `histogram_boundaries`。

### 2. 记录

只通过现有 `AsyncMetricSink::try_record` 发送 `MetricEvent`：

```rust
let labels = MetricLabels::one(LabelKey::Operation, "channel_sync")
    .with(LabelKey::Status, "error")
    .with(LabelKey::ErrorKind, "timeout");

let _ = metrics.try_record(MetricEvent::counter(
    MetricId::ImSyncTimeoutTotal,
    1.0,
    labels,
));
```

MessageV3 业务指标放在 canonical event/terminal 分类器；只有 matching `PersistOk` 后的真实
终态才能计为业务成功。HTTP 2xx、WS 入队、socket write、accepted 或内存 mutation 不能代替
terminal event。

### 3. 证明

为注册和 producer 增加 focused test；若有看板需求，再更新 `dashboard.contract.json`、中文
标题表并运行生成器和 contract checker。不要手改生成的 dashboard JSON。

### 4. 保持低侵入

- 业务线程只做 bounded、非阻塞的 `try_record`；不 await OTLP、不做 DNS/TLS/HTTP、不要同步 flush。
- 不新增无界 channel、锁竞争、逐事件网络回调或每次记录的 `String`/JSON 分配。
- 不在 `helix-core`、`helix-im` 引入 OTel/Prometheus runtime；driver 才拥有 exporter、线程和
  生命周期。
- 标签最多 8 个且值域有界；禁止 `corr_key`、run ID、user/channel/post ID、token、cookie、完整 URL。
- 失败时允许 Noop/Drop，但必须保留 exporter 自观测；不得改变主业务成功/失败语义。

## L3：分层验证与结论

按顺序执行，缺一层不得越级宣称 Green：

1. **FOCUSED**：`cargo test -p helix-driver-host --test business_metrics`、
   `cargo test -p helix-driver-host --test metrics_runtime`；证明注册、分类、队列、重试和熔断，
   不证明 Collector 收到。
2. **CONFIG/STATIC**：检查 YAML、Collector pipeline、Prometheus scrape、Grafana contract；
   只证明配置或生成物。
3. **EXPORT**：在受控本地/目标运行态观察 `HELIX_METRICS_EXPORT_OK`、exporter state 与无错误
   的 bounded worker；只证明 exporter 尝试/成功，不单独证明业务样本。
4. **COLLECTOR_SCRAPE**：查询 Collector Prometheus exporter `:9464/metrics`，确认目标指标出现。
5. **PROM_QUERY**：运行 `scripts/verify_prometheus_metric.py`，同时检查 `/api/v1/metadata` 和
   `/api/v1/query`，要求 query `status=success`、结果非空、样本时间新鲜且 service/resource
   标签匹配。
6. **REAL_BUSINESS**：在同一有界时间窗口触发真实业务事件，再复查 Collector、Prometheus、
   service name、deployment environment 和目标 series。只读验证脚本不发送业务请求、不写库、
   不调用 `/v1/metrics` 注入 synthetic canary。

结论使用 `PASS`、`PARTIAL`、`BLOCKED` 或 `UNVERIFIED`，并逐条写明证据和不能证明什么。

## 失败分类

- `BLOCKED_REVIEW`：没有独立或用户的 `REVIEW: APPROVED`。
- `CONTRACT_DRIFT`：名称、类型、单位、owner、终态或标签合同与当前代码不一致。
- `HOT_PATH_RISK`：阻塞、无界队列、额外 I/O、锁竞争或明显分配进入主路径。
- `HIGH_CARDINALITY_LABEL`：标签含实体 ID、correlation、凭据或未封闭值域。
- `NO_PRODUCER`：只注册没有任何真实 `try_record` producer。
- `COLLECTOR_UNAVAILABLE`：Collector/Prometheus 不可达或运行态不是当前目标。
- `PROM_METADATA_MISSING`：Prometheus 尚未抓到该指标的 metadata。
- `PROM_QUERY_EMPTY`：查询成功但没有近期 series/sample。
- `STALE_SAMPLE`：有 series 但样本超出评审窗口，不能证明本轮采集。
- `CONFIG_ONLY`：只有静态配置、build 或 health/HTTP 200 证据。
- `CANARY_ONLY`：只有 telemetry canary，不能证明新业务指标。

## 输出合同

交付报告必须包含：

```text
agent: helix-otel-prometheus-metrics-agent
status: needs_review | blocked | partial | done
review: PROPOSED | APPROVED | CHANGES_REQUESTED
changed_files: [...]
metric_contract: {...}
evidence: [{level, command_or_source, result, proves, does_not_prove}]
unverified: [...]
risks: [...]
next_step: ...
```

没有真实 Collector/Prometheus 运行态时，把 `status` 降为 `partial` 或 `blocked`，不要把静态
检查写成采集成功。
