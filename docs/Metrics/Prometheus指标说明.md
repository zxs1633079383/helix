# Helix Runtime Prometheus 指标说明

> 环境部署、Recording Rule 选型、迁移顺序与分层验收见
> [`MessageV3 全链路观测环境迁移手册`](./MessageV3全链路观测环境迁移手册.md)。

> **当前基线**：`reactor/MessageV3-helix`，核对日期：2026-08-24。
>
> 本文只描述当前代码中 `MetricAvailability::Runtime` 的指标。当前注册表共有 **176** 个
> 指标，其中 **122 个 Runtime**；另外 41 个是 `Planned`、11 个是 `HarnessOnly`、2 个是
> `ProfileOnly`。这里的 122 是“常规运行时允许采集”的精确分母，不表示每个业务路径在每个
> 抓取窗口都会产生样本。
>
> 指标目录与可用性真源是 [`registry.rs`](../../crates/helix-driver-host/src/metrics/registry.rs)。
> 如果指标 exporter 没有启动、OTLP Collector 不可达或当前路径没有触发 recorder，Prometheus
> 可能没有对应样本；不能用“注册过”代替“真实运行已采集”。

## 1. 暴露链路与口径

Helix 不直接实现 Prometheus remote-write，也不在 `helix-core` 中建立网络 exporter。当前
`helix-driver-host` 的路径是：

```text
Helix host/native/FFI
    └─ 有界 try_record（业务线程只入队，不等待网络）
        └─ 独立 metrics worker
            └─ OTLP gRPC :4317
                └─ OpenTelemetry Collector
                    └─ Prometheus exporter / scrape
```

OTLP Metrics pipeline 在 [`otlp.rs`](../../crates/helix-driver-host/src/metrics/otlp.rs) 中创建
所有注册 Instrument；有界队列、批量、重试、熔断和关闭在 [`runtime.rs`](../../crates/helix-driver-host/src/metrics/runtime.rs)
与 [`worker.rs`](../../crates/helix-driver-host/src/metrics/worker.rs) 中执行。业务线程不会同步
调用 OTLP、DNS、TLS 或 Prometheus 网络路径。

### 1.1 当前分母

| availability | 数量 | 是否属于本文 | 说明 |
|---|---:|---|---|
| `Runtime` | **122** | 是 | 正常 host/native/FFI 运行中可由真实 adapter 记录 |
| `Planned` | 41 | 否 | 名称/单位合同已登记，当前没有完整生产采集 seam |
| `HarnessOnly` | 11 | 否 | 只在有限时长压测或正确性 Harness 收尾时发布 |
| `ProfileOnly` | 2 | 否 | 只在 allocator/profile 模式发布 |
| 注册总数 | **176** | 否 | schema/catalog 总量 |

旧测试中的 `ALL_METRIC_IDS.len() > 100` 与 `production_metric_ids().count() > 80` 只是下限断言，
不能当作当前精确数量。

### 1.2 低基数标签合同

指标只允许使用 [`labels.rs`](../../crates/helix-driver-host/src/metrics/labels.rs) 中的白名单
键，常见值包括：

- 资源维度：`deployment_environment`、`platform`、`scenario`、`channel_shape`、`profile`。
- 执行维度：`stage`、`tick_kind`、`effect_kind`、`storage_op`、`operation`、`direction`、
  `protocol`、`status`、`status_class`、`error_kind`、`lifecycle_state`。
- 适配器维度：`consumer`、`pool`、`overflow`、`exporter_state`。

`lifecycle_id`、`operation_id`、`corr_key`、`runId`、`userId`、`companyId`、`channelId`、
`messageId`、token、cookie、完整 URL 和 SQL 参数不进入 Prometheus label。它们属于 Trace 或
独立的 `full_debug` capture；否则会造成高基数、泄漏敏感信息并污染时序库。

### 1.3 指标类型

| 类型 | 用途 | Prometheus 常用查询 |
|---|---|---|
| `Counter` | 单调累计的请求、错误、丢弃或完成次数 | `rate()`、`increase()` |
| `Gauge` | 当前队列、在途、状态、容量或资源值 | 当前值、`avg_over_time()`、`max_over_time()` |
| `Histogram` | 延迟、大小、批量和放大分布 | `histogram_quantile()`、P50/P95/P99 |

Collector 可能为 OTel 指标增加 `otel_` 前缀或把 Resource 转成 Prometheus label；看板必须以
Prometheus `/api/v1/metadata` 的真实名称为准，本文列出的 `helix_*` 是 Helix canonical 名称。

## 2. Runtime 指标分类总览

| 分类 | 数量 | 主要 owner |
|---|---:|---|
| 核心操作与 Tick 调度 | 17 | `engine/perf_metrics.rs`、`tick_ingress.rs` |
| Effect 与 Worker Pool | 16 | `engine/perf_metrics.rs`、`dispatch.rs`、`spawner.rs` |
| HTTP | 7 | `network/http.rs`、`dispatch.rs` |
| Persist 与 SQLite Storage | 8 | `dispatch.rs`、`storage.rs` |
| PortReply 与 correlation | 9 | `engine/perf_metrics.rs` |
| WebSocket | 10 | `engine/perf_metrics.rs`、`dispatch.rs` |
| Command milestone | 5 | `engine/perf_metrics.rs` |
| Event bus | 7 | `dispatch.rs`、native/FFI event sink |
| MessageV3 业务终态 | 6 | `metrics/business.rs`、`dispatch.rs` |
| Timer | 7 | `timer.rs` |
| FFI | 7 | `helix-driver-ffi/src/event_sink.rs`、`api/command.rs` |
| Process 与 shutdown | 3 | `engine/perf_metrics.rs` |
| Metrics exporter 自观测 | 8 | `metrics/runtime.rs`、`metrics/worker.rs` |
| Trace exporter 自观测 | 11 | `otel/exporter.rs` |
| Telemetry canary | 1 | `metrics/worker.rs` |
| **合计** | **122** | |

## 3. 各 Runtime 指标说明

### 3.1 核心操作与 Tick 调度（17）

采集 owner：[`engine/perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)、
[`tick_ingress.rs`](../../crates/helix-driver-host/src/tick_ingress.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_operations_total` | Counter / `operations` | Host 已观测到的 operation 次数，按 `operation`、`stage`、`status` 分解。 |
| `helix_errors_total` | Counter / `errors` | 跨 core、HTTP、storage、dispatch 的错误总计；具体根因放在低基数 `error_kind`。 |
| `helix_command_accepted_total` | Counter / `1` | Command 成功进入 Tick ingress 队列。 |
| `helix_command_rejected_total` | Counter / `1` | Command 在 ingress 阶段被拒绝，例如队列不可接收。 |
| `helix_command_admission_duration_seconds` | Histogram / `s` | 从 command 提交到 ingress 接受或拒绝的等待时延。 |
| `helix_ticks_total` | Counter / `1` | Engine 开始处理一个 Tick；可按 `tick_kind` 区分 inbound、command、timer、port reply。 |
| `helix_core_step_duration_seconds` | Histogram / `s` | core 对单个 Tick 执行 step 的耗时。 |
| `helix_core_step_errors_total` | Counter / `1` | core step 返回错误的次数。 |
| `helix_core_empty_effect_total` | Counter / `1` | Tick 完成但没有产生 Effect 的次数，用于发现空转或无效输入。 |
| `helix_tick_queue_wait_seconds` | Histogram / `s` | Tick 从入队到被 Engine 取出之间的排队时延。 |
| `helix_tick_queue_depth` | Gauge / 无量纲 | Tick ingress 队列当前积压数量。 |
| `helix_tick_queue_capacity` | Gauge / 无量纲 | Tick ingress 队列配置容量。 |
| `helix_tick_queue_full_total` | Counter / `1` | Tick ingress 队列满导致拒绝的次数。 |
| `helix_tick_inflight` | Gauge / `1` | 当前正在执行的 Tick 数；正常单 Engine 通常为 0 或 1。 |
| `helix_reply_fairness_forced_total` | Counter / `1` | PortReply 洪峰下触发 ingress 公平性闸门的次数，防止新输入长期饥饿。 |
| `helix_engine_loop_iteration_seconds` | Histogram / `s` | Engine 一轮完整循环耗时，覆盖 step、dispatch 与事件 flush。 |
| `helix_engine_state` | Gauge / `state` | Engine 运行状态；当前 `1` 表示 running，`0` 表示 stopped。 |

### 3.2 Effect 与 Worker Pool（16）

采集 owner：[`perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)、
[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)、
[`spawner.rs`](../../crates/helix-driver-host/src/spawner.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_effects_total` | Counter / `1` | Engine dispatch 的 Effect 数量，按 `effect_kind` 统计。 |
| `helix_effects_per_tick` | Histogram / `1` | 每个 Tick 产生的 Effect 个数分布。 |
| `helix_effect_bytes_per_tick` | Histogram / `bytes` | 每个 Tick 的 Effect 编码体积分布。 |
| `helix_effect_amplification_ratio` | Histogram / `ratio` | Effect 批次放大口径；当前 recorder 以批次 summary 的 count 写入，需结合同一窗口的输入量解读。 |
| `helix_effect_dispatch_duration_seconds` | Histogram / `s` | 单个 Effect 从 dispatch 到 adapter 返回的耗时。 |
| `helix_effect_dispatch_errors_total` | Counter / `1` | Effect adapter 执行失败次数。 |
| `helix_pool_queue_depth` | Gauge / `1` | Worker Pool 当前等待执行的任务数。 |
| `helix_pool_queue_capacity` | Gauge / `1` | Worker Pool 队列容量。 |
| `helix_pool_enqueue_block_seconds` | Histogram / `s` | 任务进入 Worker Pool 时遭遇背压的阻塞时延。 |
| `helix_pool_queue_residency_seconds` | Histogram / `s` | 任务在 Worker Pool 队列中的驻留时延。 |
| `helix_pool_execution_seconds` | Histogram / `s` | Worker 实际执行任务的耗时。 |
| `helix_pool_inflight` | Gauge / `1` | 当前正在执行的 Worker 任务数。 |
| `helix_pool_workers` | Gauge / `1` | 当前 Worker 数量。 |
| `helix_pool_dropped_total` | Counter / `1` | Worker 任务因队列或生命周期原因被丢弃的次数。 |
| `helix_pool_closed_total` | Counter / `1` | Pool 已关闭后仍有任务提交的次数。 |
| `helix_pool_job_panics_total` | Counter / `1` | Worker job panic 次数；出现增长表示任务隔离仍需排查。 |

### 3.3 HTTP（7）

采集 owner：[`network/http.rs`](../../crates/helix-driver-host/src/network/http.rs)、
[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_http_queue_wait_seconds` | Histogram / `s` | HTTP Effect 从 dispatch 入队到 HTTP worker 开始处理的等待时延。 |
| `helix_http_request_duration_seconds` | Histogram / `s` | HTTP request 从发送到收到并读取响应或失败的总耗时。 |
| `helix_http_inflight` | Gauge / 无量纲 | 当前在途 HTTP 请求数。 |
| `helix_http_request_bytes` | Histogram / `bytes` | HTTP 请求 body 字节数分布。 |
| `helix_http_response_bytes` | Histogram / `bytes` | HTTP 响应 body 字节数分布。 |
| `helix_http_timeout_total` | Counter / `1` | 被识别为 timeout 的 HTTP 失败次数。 |
| `helix_http_status_total` | Counter / `1` | 收到 HTTP response 的次数，按 `status_class`（1xx/2xx/3xx/4xx/5xx/other）统计。 |

### 3.4 Persist 与 SQLite Storage（8）

采集 owner：[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)、
[`storage.rs`](../../crates/helix-driver-host/src/storage.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_persist_queue_wait_seconds` | Histogram / `s` | Persist Effect 在 dispatch 队列中等待进入 SQLite 操作的时延。 |
| `helix_storage_tx_duration_seconds` | Histogram / `s` | 一次 SQLite storage operation/transaction 的总耗时。 |
| `helix_storage_rows_total` | Counter / `rows` | 成功写入、更新或删除的行数累计。 |
| `helix_storage_inflight` | Gauge / `1` | 当前在途 SQLite 操作数。 |
| `helix_storage_busy_total` | Counter / `1` | SQLite 返回 busy/locked 的次数。 |
| `helix_storage_timeout_total` | Counter / `1` | SQLite 操作超时次数。 |
| `helix_storage_rollback_total` | Counter / `1` | `atomic_write` 失败触发回滚语义的次数。 |
| `helix_storage_batch_rows` | Histogram / `rows` | 单次 storage batch 处理的行数分布。 |

### 3.5 PortReply 与 correlation（9）

采集 owner：[`engine/perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)。
Port correlation 表使用固定容量和 O(1) 槽位，不把 `corr_key` 放进 Prometheus label。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_port_roundtrip_seconds` | Histogram / `s` | Effect 发出到匹配 PortReply 返回的往返时延。 |
| `helix_port_reply_queue_depth` | Gauge / `1` | feedback/PortReply channel 当前积压数量。 |
| `helix_port_reply_queue_wait_seconds` | Histogram / `s` | PortReply 在 feedback queue 中的等待时延。 |
| `helix_port_reply_total` | Counter / `1` | 成功匹配 correlation 的 PortReply 数，按 `status` 区分 ok/error。 |
| `helix_port_reply_orphan_total` | Counter / `1` | 找不到对应 pending slot 的 PortReply 次数。 |
| `helix_port_correlation_collision_total` | Counter / `1` | 固定 correlation slot 被新 Effect 覆盖的次数。 |
| `helix_port_pending` | Gauge / `1` | 当前等待 PortReply 的 correlation 数量。 |
| `helix_port_pending_capacity` | Gauge / `1` | correlation 表固定容量，当前实现为 4096。 |
| `helix_unknown_port_reply_total` | Counter / `1` | 收到未知或无法匹配的 PortReply 次数；通常与 orphan 一起排查。 |

### 3.6 WebSocket（10）

采集 owner：[`engine/perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)、
[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_ws_connection_state` | Gauge / `state` | 当前是否至少有一条 WebSocket transport；当前 `1` 表示有连接，`0` 表示无连接。 |
| `helix_ws_connect_total` | Counter / `1` | WebSocket Connected Tick 次数。 |
| `helix_ws_disconnect_total` | Counter / `1` | WebSocket Disconnected Tick 次数。 |
| `helix_ws_frames_total` | Counter / `1` | 收到或发送的 WebSocket frame 数，按 `direction` 区分。 |
| `helix_ws_frame_bytes` | Histogram / `bytes` | WebSocket frame 字节数分布。 |
| `helix_ws_send_duration_seconds` | Histogram / `s` | Effect 发送 WebSocket frame 的耗时。 |
| `helix_ws_send_errors_total` | Counter / `1` | WebSocket frame 发送失败次数。 |
| `helix_ws_inbound_last_seen_age_seconds` | Gauge / `s` | 最近一次 inbound frame 距当前的年龄；收到新 frame 时归零。 |
| `helix_ws_ingress_to_effect_seconds` | Histogram / `s` | inbound frame 进入 Tick 到 core 产生 Effect 的时延。 |
| `helix_ws_ingress_to_event_seconds` | Histogram / `s` | inbound frame 进入 Tick 到事件 flush 的端到端时延。 |

### 3.7 Command milestone（5）

采集 owner：[`engine/perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)。
这些指标描述同一 command 根在不同完成里程碑的耗时，不把 command/lifecycle ID 作为 label。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_command_to_persist_reply_seconds` | Histogram / `s` | Command 开始到 Persist PortReply 返回的耗时。 |
| `helix_command_to_immediate_projection_seconds` | Histogram / `s` | Command 开始到无需 PortReply 的即时 projection emit 的耗时。 |
| `helix_command_to_http_dispatch_seconds` | Histogram / `s` | Command 开始到 HTTP Effect 被 dispatch 的耗时。 |
| `helix_command_to_http_response_seconds` | Histogram / `s` | Command 开始到 HTTP PortReply/response 返回的耗时。 |
| `helix_command_to_projection_emit_seconds` | Histogram / `s` | Command 经 PortReply 后到 authoritative projection emit 的耗时。 |

### 3.8 Event bus（7）

采集 owner：[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)、
[`helix-driver-native/src/event_sink.rs`](../../crates/helix-driver-native/src/event_sink.rs)、
[`helix-driver-ffi/src/event_sink.rs`](../../crates/helix-driver-ffi/src/event_sink.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_event_lagged_total` | Counter / `1` | Native broadcast consumer 读取时发现 lagged、被跳过的事件数。 |
| `helix_event_no_receiver_total` | Counter / `1` | Event emit 时没有任何 receiver 的次数。 |
| `helix_event_batch_size` | Histogram / `1` | 一个 Tick/flush 中准备交付的领域事件数量。 |
| `helix_event_emitted_total` | Counter / `1` | EventSink 成功进入事件发布路径的次数。 |
| `helix_event_emit_duration_seconds` | Histogram / `s` | Event emit/flush 路径耗时。 |
| `helix_event_receiver_count` | Gauge / `1` | 当前 Event bus receiver 数量。 |
| `helix_event_consumer_closed_total` | Counter / `1` | Consumer 发现 broadcast channel closed 的次数。 |

### 3.8.1 MessageV3 业务终态（6 Runtime + 1 Planned）

采集 owner：[`business.rs`](../../crates/helix-driver-host/src/metrics/business.rs)、
[`dispatch.rs`](../../crates/helix-driver-host/src/dispatch.rs)。采集点位于稳定
`OwnedEffect::Emit` 边界，仅按固定事件名映射低基数终态；不解析或导出消息正文、用户 ID、频道 ID、
消息 ID 与 correlation key。未知事件和中间态不产生伪终态。

| Prometheus 指标 | availability | 类型 / 单位 | 对应功能 |
|---|---|---|---|
| `helix_im_projection_terminal_total` | Runtime | Counter / `1` | Helix 权威投影成功或失败终态，按固定 `operation/status/error_kind` 分解。 |
| `helix_im_sync_session_total` | Runtime | Counter / `1` | 同步会话稳定成功或异常终态；不能用“开始拉取”代替成功。 |
| `helix_im_sync_anomaly_total` | Runtime | Counter / `1` | `too_long`、increment apply 失败等同步异常。 |
| `helix_im_recovery_session_total` | Runtime | Counter / `1` | offline recovery、gap repair、channel hydration 的真实完成终态。 |
| `helix_im_message_correctness_total` | Runtime | Counter / `1` | 生产路径实际检测到的 duplicate/order/gap 等正确性结果。 |
| `helix_im_command_terminal_total` | Runtime | Counter / `1` | 业务 Command 稳定成功或失败终态；不统计 admission accepted。 |
| `helix_im_unread_reconcile_total` | Planned | Counter / `1` | 必须在同 scope 获得远端绝对未读值与本地绝对值后才可记录 match/mismatch；当前不造样本。 |

### 3.9 Timer（7）

采集 owner：[`timer.rs`](../../crates/helix-driver-host/src/timer.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_timer_active` | Gauge / `1` | 当前注册且未完成/取消的 timer 数。 |
| `helix_timer_scheduled_total` | Counter / `1` | Timer 成功注册次数。 |
| `helix_timer_cancelled_total` | Counter / `1` | Timer 被取消次数，按取消原因分类。 |
| `helix_timer_fired_total` | Counter / `1` | Timer 到期并投递 Timer Tick 的次数。 |
| `helix_timer_lateness_seconds` | Histogram / `s` | 实际触发时间相对计划时间的迟到量。 |
| `helix_timer_delivery_wait_seconds` | Histogram / `s` | Timer 到期到 Timer Tick 被送入 Engine 的等待时延。 |
| `helix_timer_delivery_failed_total` | Counter / `1` | Timer 到期但 delivery 失败的次数。 |

### 3.10 FFI（7）

采集 owner：[`helix-driver-ffi/src/event_sink.rs`](../../crates/helix-driver-ffi/src/event_sink.rs)、
[`helix-driver-ffi/src/api/command.rs`](../../crates/helix-driver-ffi/src/api/command.rs)。
FFI 指标只观察批量边界和 callback 健康，不把单个事件或用户信息做 label。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_ffi_batch_events` | Histogram / `1` | 一次 C++ callback 批量交付的事件数量。 |
| `helix_ffi_batch_bytes` | Histogram / `bytes` | 一次 FFI 批量编码 buffer 的字节数。 |
| `helix_ffi_callback_duration_seconds` | Histogram / `s` | C++ host callback 执行耗时。 |
| `helix_ffi_busy_total` | Counter / `1` | FFI command 因已有 command 在执行而返回 busy 的次数。 |
| `helix_ffi_callback_errors_total` | Counter / `1` | callback 缺失或调用失败导致的 FFI 错误次数。 |
| `helix_ffi_events_dropped_total` | Counter / `1` | 因 callback 不可用等边界故障而丢弃的事件数。 |
| `helix_ffi_boundary_bytes` | Histogram / `bytes` | 实际跨 Rust/C++ 边界借出的一次性 buffer 大小。 |

### 3.11 Process 与 shutdown（3）

采集 owner：[`engine/perf_metrics.rs`](../../crates/helix-driver-host/src/engine/perf_metrics.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_process_resident_memory_bytes` | Gauge / `bytes` | 独立 RSS sampler 读取到的进程常驻内存。 |
| `helix_transport_count` | Gauge / `1` | 当前已连接的 transport 数量，不携带 transport ID。 |
| `helix_shutdown_drain_seconds` | Histogram / `s` | 关闭开始到 metrics/trace/worker drain 完成的耗时。 |

### 3.12 Metrics exporter 自观测（8）

采集 owner：[`runtime.rs`](../../crates/helix-driver-host/src/metrics/runtime.rs)、
[`worker.rs`](../../crates/helix-driver-host/src/metrics/worker.rs)。这些指标用于判断“业务看起来
正常，但指标管线已经断流”的情况。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_metrics_queue_depth` | Gauge / 无量纲 | metrics worker 输入队列当前深度。 |
| `helix_metrics_queue_capacity` | Gauge / 无量纲 | metrics worker 输入队列容量。 |
| `helix_metrics_dropped_total` | Counter / `1` | 指标因 queue full 或 circuit open 被丢弃的次数，按 `error_kind` 区分。 |
| `helix_metrics_export_batch_size` | Histogram / `1` | 每次 OTLP Metrics export 的批量大小。 |
| `helix_metrics_export_duration_seconds` | Histogram / `s` | 一次 metrics export（含重试窗口）的耗时。 |
| `helix_metrics_export_errors_total` | Counter / `1` | OTLP Metrics export 失败次数。 |
| `helix_metrics_exporter_state` | Gauge / `state` | exporter 状态：`0=Disabled`、`1=Running`、`2=Backoff`、`3=Open`、`4=Stopped`。 |
| `helix_metrics_last_success_age_seconds` | Gauge / `s` | 距离最近一次成功 Metrics export 的时间；持续增长说明 telemetry 断流。 |

### 3.13 Trace exporter 自观测（11）

采集 owner：[`otel/exporter.rs`](../../crates/helix-driver-host/src/otel/exporter.rs)。这些是 Trace
管线的 Metrics，不是 Trace span 本身。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_traces_queue_depth` | Gauge / `1` | trace worker 待导出的 span 队列深度。 |
| `helix_traces_queue_capacity` | Gauge / `1` | trace worker 队列容量。 |
| `helix_traces_dropped_total` | Counter / `1` | trace 队列满或 exporter 不可用时丢弃的 span 数。 |
| `helix_traces_export_batch_size` | Histogram / `1` | 每次 OTLP Trace export 的 span 批量大小。 |
| `helix_traces_export_duration_seconds` | Histogram / `s` | 一次 span batch export 的耗时。 |
| `helix_traces_export_errors_total` | Counter / `1` | span batch 重试后仍失败的次数。 |
| `helix_traces_exporter_state` | Gauge / `state` | trace exporter 状态；当前实现 `0=stopped/absent`、`1=running/last success`、`2=export failure/backoff`。 |
| `helix_traces_last_success_age_seconds` | Gauge / `s` | 距离最近一次成功 Trace export 的时间。 |
| `helix_spans_created_total` | Counter / `1` | Helix 创建 span 的数量。 |
| `helix_spans_sampled_total` | Counter / `1` | 进入采样/导出队列的 span 数。 |
| `helix_slow_spans_total` | Counter / `1` | 被标记为 `helix.slow=true` 的慢 span 数。 |

### 3.14 Telemetry canary（1）

采集 owner：[`worker.rs`](../../crates/helix-driver-host/src/metrics/worker.rs)。

| Prometheus 指标 | 类型 / 单位 | 对应功能 |
|---|---|---|
| `helix_telemetry_canary_total` | Counter / `1` | metrics worker 成功走过一次自观测批次的心跳；用于先验证 Collector/Prometheus 链路，再判断业务指标。 |

## 4. 当前不属于 Runtime 的指标

下列指标已在 registry 中登记，但本文件不把它们写成常规 Prometheus 运行指标：

- `Planned` 41 个：例如未读权威对账、HTTP cancelled、storage pool/WAL/checkpoint、WS reconnect/decode、
  command-to-projection consume、FFI command duration、process uptime、shutdown timeout 等。
- `HarnessOnly` 11 个：`helix_lost_total`、`helix_duplicate_total`、顺序/频道/cursor/projection
  正确性及压测吞吐指标；它们只在有界验收窗口发布。
- `ProfileOnly` 2 个：`helix_alloc_count_per_op`、`helix_alloc_bytes_per_op`。

另一个尚未合入当前 `reactor/MessageV3-helix` 基线的 full_debug 变更增加了
`helix_traces_debug_queue_depth`、`helix_traces_debug_queue_capacity`、
`helix_traces_debug_chunks_dropped_total` 三个指标；合入后应把本文分母从 176/122 更新为
179/125，并补充对应的独立 debug chunk/drop 语义。当前基线不应提前把它们写成已暴露指标。

## 5. 最小 Prometheus 验收查询

以下查询只能证明 Prometheus 收到了样本，不能替代 Helix 业务真实链路证据；查询前应以
`/api/v1/metadata` 确认 Collector 的实际名称是否带 `otel_` 前缀。

```promql
# 业务操作速率
sum by (stage, operation, status) (rate(helix_operations_total[5m]))

# 错误速率
sum by (stage, error_kind) (rate(helix_errors_total[5m]))

# HTTP P95
histogram_quantile(
  0.95,
  sum by (le) (rate(helix_http_request_duration_seconds_bucket[5m]))
)

# Metrics exporter 是否断流
helix_metrics_exporter_state
helix_metrics_last_success_age_seconds
rate(helix_metrics_export_errors_total[5m])

# Trace exporter 是否丢 span
rate(helix_traces_dropped_total[5m])
rate(helix_traces_export_errors_total[5m])

# canary 是否持续到达
rate(helix_telemetry_canary_total[5m])
```

## 6. 变更维护规则

1. 新增、删除或调整指标时，先改 [`registry.rs`](../../crates/helix-driver-host/src/metrics/registry.rs)，
   再同步本文的分母、分类、名称、类型、单位和采集 owner。
2. 只在真实 recorder 已接入且跨平台边界不破坏的情况下把 `Planned` 改为 `Runtime`。
3. 高基数实体信息始终进入 Trace/full_debug，不得为了看板方便加入 Metrics label。
4. 文档中的数字必须带 revision；分支或 worktree 未合并的指标只能放在“未纳入当前基线”中。
5. Prometheus 样本验收必须绑定真实运行窗口、service/resource label 和 query 结果；注册表单元测试
   不能单独升级为 Prometheus runtime PASS。
