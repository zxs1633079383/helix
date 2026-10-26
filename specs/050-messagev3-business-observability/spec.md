# MessageV3 核心业务可观测性规格

状态：`IMPLEMENTING / REAL_EVIDENCE_NOT_RUN`

创建时间：2026-08-24

实现范围：GoServer、Helix host metrics、Prometheus recording rules、Grafana 中文仪表盘。

## Problem Statement

当前系统已经暴露大量 Go runtime、连接池、Pulsar、WebSocket、Helix engine、存储和 OTLP
运行时指标，但缺少一组口径稳定、可以直接回答“消息是否可靠完成”的业务原子指标。现有
`HTTP 2xx`、事务阶段耗时、Hub 入队、Socket 写入、`Effect::Emit` 等观测点属于不同证据层，
不能彼此替代。若直接在 Grafana 中临时拼接这些指标，团队会得到多个口径不同的“送达率”或
“成功率”，无法稳定告警，也无法判断失败发生在远端持久化、Outbox、扇出、客户端接收、
本地投影还是同步恢复。

## Solution

建立十二项 MessageV3 核心业务指标，以“各 Owner 暴露低基数原子指标，Prometheus recording
rules 统一计算比率和分位数，Grafana 只展示冻结口径”为原则：

1. 消息远端落库成功率；
2. Outbox 最终投递成功率；
3. 服务端每接收人入队成功率；
4. 客户端真实 ACK 送达率；
5. Helix 本地权威投影成功率；
6. 消息端到端延迟 P50/P95/P99；
7. 同步会话成功率；
8. 同步异常会话率；
9. 离线/断线恢复成功率；
10. 未读对账不一致率；
11. 消息丢失、重复、乱序率；
12. 所有业务 Command 终态成功率。

指标只在真实业务终态产生样本。缺少真实终态的能力先冻结合同并保持无样本，不使用代理指标
伪造结果。第一阶段不改变 MessageV3 业务 payload，不让 `helix-im` 依赖运行时指标库；Helix
业务事件由 host 在 `Effect::Emit` 边界进行固定前缀、无分配分类。

## User Stories

1. 作为值班工程师，我希望看到消息远端事务提交成功率，以便区分业务失败和数据库提交失败。
2. 作为值班工程师，我希望看到 Outbox 创建、最终标记投递和重试失败的终态，以便识别提交后未传播的消息。
3. 作为服务端工程师，我希望每个合资格接收连接都有唯一入队终态，以便发现慢消费者和连接缺失。
4. 作为客户端工程师，我希望只有真实客户端 ACK 才进入送达率分子，以便避免把 Socket 写成功误称为用户收到。
5. 作为 Helix 工程师，我希望权威消息在本地提交后产生投影成功样本，以便发现 WS 已到达但本地事实未完成的问题。
6. 作为 SRE，我希望查看消息各阶段和可闭合端到端链路的 P50/P95/P99，以便定位尾延迟。
7. 作为同步模块 Owner，我希望每轮同步会话只记录一个成功或失败终态，以便得到稳定分母。
8. 作为同步模块 Owner，我希望 TooLong、gap、cursor regression、重复、丢失、乱序和 projection mismatch 有稳定异常分类。
9. 作为客户端工程师，我希望断线、离线和 authority recovery 会话有成功、失败和耗时，以便衡量自愈能力。
10. 作为业务工程师，我希望未读值发生权威对账时记录 match 或 mismatch，以便发现多端未读漂移。
11. 作为质量工程师，我希望消息丢失、重复和乱序使用业务会话或业务消息作为稳定分母，以便发现低频严重故障。
12. 作为平台工程师，我希望所有业务 Command 使用统一 operation/result/error_kind 终态合同，以便跨 Gate 比较成功率。
13. 作为 Grafana 使用者，我希望所有面板标题、说明和图例使用中文，以便团队直接理解口径。
14. 作为 Prometheus 管理员，我希望指标标签保持低基数，以便业务指标不会拖垮时序数据库。
15. 作为排障工程师，我希望 correlation、用户、频道、消息和序号进入日志/Trace 而不是指标标签，以便既能关联又不产生基数爆炸。
16. 作为发布评审者，我希望每项指标注明 Runtime、Planned 或 HarnessOnly，以便缺少数据时不会误判为零故障。
17. 作为发布评审者，我希望 recording rule 的分母只包含相同业务实体的终态，以便成功率可以被证伪。
18. 作为开发者，我希望通过现有 Prometheus registry 和 Helix AsyncMetricSink 接入，以便降低侵入和保持架构边界。
19. 作为开发者，我希望指标故障永远不阻断消息主链路，以便观测系统故障不会变成业务故障。
20. 作为测试工程师，我希望聚焦测试能从公开注册表采集指标并核对 labels/value，以便不依赖实现细节。
21. 作为测试工程师，我希望真实链验收能用同一 run_id/corr 对账 HTTP、WS、远端提交、本地投影和 EventSink，以便分层证据不串轮次。
22. 作为客户端产品 Owner，我希望 ACK 协议未实现时面板明确显示“无真实 ACK 数据”，而不是展示虚假的 100%。
23. 作为运维人员，我希望 Prometheus 预计算核心比率和分位数，以便多个 Grafana 面板复用同一口径。
24. 作为容量工程师，我希望原子指标保留 stage 和 platform 维度，以便比较 Go、Native、FFI 和 Web 的表现。

## Implementation Decisions

### Owner 与原子指标合同

| 核心指标 | Owner | 原子指标 | 当前可用性 |
|---|---|---|---|
| 消息远端落库成功率 | GoServer | `im_message_persist_terminal_total{result,error_kind}` | Runtime |
| Outbox 最终投递成功率 | GoServer | `im_delivery_outbox_lifecycle_total{stage,result,error_kind}` | Runtime |
| 每接收人入队成功率 | GoServer WS | 既有 `im_ws_hub_delivery_events_total{result,event_type}` | Runtime，复用真实 per-connection deliver seam |
| 客户端真实 ACK 送达率 | 客户端 + GoServer | `im_message_client_ack_total{platform,result}` | Planned；当前无业务 ACK action |
| Helix 本地权威投影成功率 | Helix host | `helix_im_projection_terminal_total{operation,status,error_kind}` | Runtime |
| 消息端到端延迟 | 各 Owner + Prometheus | `im_message_lifecycle_duration_seconds{stage,result}` 与现有 `helix_command_to_projection_emit_seconds` | 阶段 Runtime；跨进程全程延迟在 ACK 前 Planned |
| 同步会话成功率 | Helix host | `helix_im_sync_session_total{operation,status,error_kind}` | Runtime |
| 同步异常会话率 | Helix host | `helix_im_sync_anomaly_total{operation,status,error_kind}` | Runtime |
| 离线/断线恢复成功率 | Helix host | `helix_im_recovery_session_total{operation,status,error_kind}` | Runtime |
| 未读对账不一致率 | Helix host | `helix_im_unread_reconcile_total{status,error_kind}` | Planned，直到出现同一 scope 的远端/本地对账终态 |
| 丢失、重复、乱序率 | Helix host | `helix_im_message_correctness_total{error_kind,status}` | Runtime，仅真实检测点；HarnessOnly 指标不冒充 Runtime |
| Command 终态成功率 | Helix host / GoServer | `helix_im_command_terminal_total{operation,status,error_kind}`、`im_business_command_terminal_total{operation,result,error_kind}` | Runtime，按已有真实终态逐步覆盖 |

### 终态与分母

- 远端落库：一次消息写事务最终 `commit` 为 `success`；rollback、timeout、validation 或 store
  错误为失败。同一请求的 fast/normal path 只能产生一次终态。
- Outbox：记录创建成功、正常路径标记 delivered、恢复 dispatcher 标记 delivered、decode/load/
  mark 失败。当前无最大重试终态时不得命名为 `exhausted`。
- 每接收人入队：`enqueued` 为成功；`connection_missing`、`delivery_probe_rejected`、
  `slow_consumer` 为失败/拒绝。按连接而不是按消息计数，并在面板明确口径。
- 客户端 ACK：必须由客户端在本地权威投影成功后发送包含稳定消息 identity 和幂等 ACK identity
  的业务 action，GoServer 验证 audience 后记终态。Socket Write、ping/pong、deadQueue seq 和
  receiver probe 均不能替代。
- Helix 投影：只有 matching PersistOk/read-back 后产生的稳定 MessageV3 terminal event 才算成功；
  `sending`、ready、page 等中间事件不进入成功分子。
- 同步会话：每轮 channel sync、explicit sync 或 recovery epoch 只记录一次 terminal；分页和单条
  event 不能成为会话分母。
- 异常同步：异常会话进入同步会话失败终态，并另记低基数 `error_kind`；一个会话即便出现多个
  异常，也只进入异常率分子一次。
- 未读对账：必须同时存在相同 tenant/viewer/channel/snapshot 的远端绝对值和本地绝对值；只观测
  单侧未读不能产生 match/mismatch。
- 丢失/重复/乱序：仅在 Helix cursor/gate/projection 的真实检测点记录；压力测试收尾统计继续保持
  HarnessOnly，不进入常驻 Runtime 分子。
- Command：accepted/rejected 是 admission，不是终态。只有业务成功 event 或稳定失败 event 才计入。

### 标签合同

允许标签：`operation`、`stage`、`result/status`、`error_kind`、`event_type`、`mode`、
`platform`、`source`。值必须来自代码内固定枚举或固定映射。

禁止标签：`user_id`、`channel_id`、`post_id`、`message_id`、`temporary_id`、`req_id`、
`corr`、`event_seq`、`track_id`、连接 ID、原始错误文本和 URL。关联信息进入结构化日志、Trace
或 exemplar。

### Helix 边界

- `helix-core` 和 `helix-im` 不新增 Prometheus/OTel/runtime 依赖。
- host 在通用 `OwnedEffect::Emit` seam 对 canonical `{"event":"...","data":...}` 做固定前缀分类，
  不解析完整 JSON、不构造 String、不遍历 data。
- 分类器只返回静态 operation/status/error_kind；未知或非 IM event 不产生业务指标。
- MessageV3 bytes 原样交付 EventSink；指标旁路不得修改 payload、Emit 次数或批次边界。

### GoServer 边界

- 扩展现有 Prometheus registry，不引入另一套 exporter。
- 远端事务只在统一 commit seam 记录一次终态。
- Outbox 在真实 insert/mark/recovery seam 记录，不以 publish 函数返回代表最终投递。
- 每接收人入队可以从既有 Hub delivery 原子事件通过 recording rule 计算；若新增更明确别名，
  必须在同一 deliver seam 记录，避免双重业务计数。
- 指标调用不得改变事务、publish、retry 或 error 返回。

### Prometheus 与 Grafana

- Prometheus recording rules 是统一派生指标真源，Grafana 不复制复杂公式。
- 比率分母为零时返回无数据，不使用 `or vector(1)` 伪造 100%。
- 直方图分位数使用 `histogram_quantile`，至少输出 P50/P95/P99。
- Grafana 新增独立中文“MessageV3 业务可靠性”仪表盘，清晰区分 Runtime、Planned 和代理指标。
- 第一阶段允许 Grafana 同时查询 GoServer Prometheus scrape 与 Helix Collector scrape；跨数据源
  不在 Grafana 客户端做逐消息 join。

### Recording rules

冻结以下逻辑名称，具体窗口默认 5 分钟，并保留 1 小时趋势：

- `im:message_persist_success_ratio`；
- `im:delivery_outbox_success_ratio`；
- `im:recipient_enqueue_success_ratio`；
- `im:client_ack_success_ratio`；
- `helix:im_projection_success_ratio`；
- `helix:im_sync_session_success_ratio`；
- `helix:im_sync_anomaly_ratio`；
- `helix:im_recovery_success_ratio`；
- `helix:im_unread_mismatch_ratio`；
- `helix:im_message_correctness_error_ratio`；
- `helix:im_command_terminal_success_ratio`；
- 消息生命周期 P50/P95/P99 三条分位数 rule。

## Testing Decisions

- 测试只验证公开指标行为：注册后的 metric family、labels、counter/histogram 样本和 MessageV3
  Emit 输入，不直接断言私有字段。
- GoServer 先写红测证明新 metric family 不存在或终态未计数，再实现 registry 与真实调用点。
- Helix 先写分类器和 dispatch 公开 seam 红测：terminal event 产生一个业务样本；中间、未知、
  非法 event 不产生；EventSink 仍收到原始 bytes。
- 覆盖成功、失败、重复/迟到、不支持和无 metrics sink 场景，确保指标不改变业务行为。
- Outbox 覆盖正常 mark、恢复 mark、decode/load/mark 失败；测试不得把 publish 调用当 delivered。
- 同步覆盖 complete/recovered/gap-repaired/too_long 等当前稳定事件；分页和 ready 不计 terminal。
- Prometheus rules 使用 `promtool check rules`（若当前环境提供）和静态表达式断言。
- Grafana JSON 使用 JSON parser、datasource/PromQL 静态检查和中文标题检查。
- FOCUSED、LOCAL_INTEGRATION、REAL_NATIVE_CHAIN 分层报告；编译或 `/metrics` 200 不替代真实链。
- 真实链使用同一 run_id/corr 对账 Go commit、Outbox、WS、Helix PersistOk、EventSink；当前任务若
  未启动真实依赖，状态保持 `REAL_EVIDENCE_NOT_RUN`。

## Out of Scope

- 本阶段不凭空新增客户端业务 ACK 协议、未读上报协议或跨进程逐消息 Prometheus join。
- 不把现有 ping/pong、WS seq、Socket 写成功、K6 receiver probe 或 HarnessOnly 结果改名为真实 ACK。
- 不修改 MessageV3 业务 payload、Gate schema、UI reducer、远端 PostgreSQL schema 或 Helix SQLite schema。
- 不一次性为所有历史 Go API 补齐业务 Command 终态；优先覆盖发送主链和当前 MessageV3 stable events。
- 不在指标 label 中加入业务 identity，不把 Grafana 面板当作协议真源。
- 不在当前 Spec 宣称生产 SLO 已达标；阈值需基于上线后的真实基线确定。

## Further Notes

- 当前 GoServer 已有发送阶段、Outbox lag/retry、Hub delivery 和 Socket write 指标，可复用但需明确
  哪些是原子业务终态、哪些只是阶段代理。
- 当前 Helix 已有 AsyncMetricSink、OTLP exporter 和低基数 label whitelist；新增业务指标应复用
  该管线，并保持 runtime-agnostic core 不变。
- 当前实现已接入 Go 远端事务/Outbox/发送 Command、既有 WS per-connection 入队、Helix 权威投影/
  同步/恢复/正确性/本地 Command lineage，以及 PrometheusRule 与 12 面板 Grafana JSON。
- 客户端 ACK 与未读对账是本规格中两个明确的后续协议 Gate。在真实 seam 落地前，Dashboard
  显示“待接入/无数据”，不能显示 0% 或 100%。
- 本规格发布到 GitLab 后使用 `ready-for-agent` 标签；本轮编码继续由当前任务执行，不等待 issue
  再次派发。
