# MessageV3 全链路观测环境迁移手册

> 适用范围：GoServer、Helix、Tauri/Angular、OpenTelemetry Collector、Prometheus 与 Grafana。
>
> 当前环境核对日期：2026-09-01。集群资源、镜像版本和运行样本会漂移，迁移时必须重新执行本文的取证命令。

## 1. 结论

**不是每个采集环节都要配置一份 Prometheus Recording Rule。**

每个业务端只负责在真实语义终点产出原始指标；Collector 负责接收 OTLP 并暴露 Prometheus
格式；Prometheus 负责抓取原始指标，并在一个环境内集中加载一次经过评审的 Recording Rules；
Grafana 查询原始指标或规则生成的规范指标。

```text
GoServer --------------------------> /metrics -----------┐
                                                         ├─> Prometheus
Angular --IPC--> Helix --异步 OTLP--> OTel Collector ----┘       │
                                                                  ├─ Recording Rules
                                                                  └─ Grafana / Alertmanager
```

Rule 不能补出生产端没有采集的事实。原始指标没有近期样本时，Dashboard 必须显示 `No data`，
不能用 Recording Rule 伪造为 `0`、`100%` 或绿色。

## 2. 哪些情况需要 Rule

| 场景 | 是否需要 Rule | 说明 |
|---|---|---|
| 查询单个原始 Counter、Gauge | 通常不需要 | Grafana 可直接查询原始指标 |
| Collector 增加了 `otel_` 前缀，需要稳定 Dashboard 名称 | 需要 | 将实际暴露名归一为 `im_*` |
| 多实例或多来源合并 | 建议需要 | 集中定义保留哪些低基数标签 |
| 高频重复的 P50/P90/P99、送达率、错误率 | 建议需要 | 降低 Dashboard 重复计算成本并统一窗口 |
| 告警依赖的 SLI/SLO | 建议需要 | Rule 与 Alert Rule 使用相同口径 |
| 按 Channel/User/Message 精确诊断 | 不应使用常驻 Rule | 使用授权 Trace 或短 TTL 诊断平面，禁止高基数 ID 常驻 Prometheus |

因此，一套环境通常只需要一份集中管理的 `PrometheusRule` 资源，其中可包含多个已批准的
Rule Group；不是 Go、Helix、Angular 每端各部署一份，也不是每台机器或每个 Pod 各部署一份。
如果 dev、pre、prod 使用彼此独立的 Prometheus，则每个环境都要分别加载其需要的 Rule Group
一次。

## 3. 各端职责

| 环节 | 应产出的原始事实 | 暴露方式 | 迁移重点 |
|---|---|---|---|
| GoServer | HTTP 延迟/请求数、消息持久化、audience 分母、WS enqueue/断开/连接等 | Go `/metrics` 由 Prometheus 直接抓取 | 镜像版本、Service、ServiceMonitor、路由模板化标签 |
| Helix | Tick、PersistOk、Projection/Event Emit、Seq/Gap、WS 客户端生命周期、遥测丢弃 | 有界队列异步 OTLP 到 Collector | 不阻塞业务线程，队列满时丢观测而非阻塞业务 |
| Tauri/Angular | `client_received`、`client_applied`、`view_updated` | Angular 经 IPC 上报，Helix 接管并异步 OTLP | UI 不等待遥测；失败不影响业务跳转 |
| OTel Collector | 接收 OTLP、批处理、资源标签转换、Prometheus 格式出口 | Prometheus exporter endpoint | 端口、内存限制、批处理、自观测、网络策略 |
| Prometheus | 抓取原始序列、执行规范化与聚合规则 | TSDB + Query API | Target、Rule Health、保留期、容量和时钟 |
| Grafana | 总览、链路漏斗、PXX、Gap/Seq、HTTP、WS、诊断入口 | Prometheus datasource | Dashboard UID、数据源 UID、变量与 `No data` 语义 |

### 3.1 低基数边界

Prometheus 标签允许 `environment`、`platform`、`path`、`stage`、`result`、`route`、
`status_class` 等有限枚举。`channelId`、`userId`、`temporaryId`、`postId`、`eventSeq`、
token 和 cookie 不进入常驻标签。总览到群聊/用户的下钻由授权 Trace 或短 TTL 诊断查询完成。

## 4. 当前 Rule Group 的迁移状态

仓库文件 [`recording-rules.yaml`](../../perf/grafana/recording-rules.yaml) 是 Prometheus
`groups:` 片段，不是可直接 `kubectl apply` 的完整 Kubernetes 资源。

| Rule Group | 当前状态 | 迁移策略 |
|---|---|---|
| `messagev3-fullchain-observability` | 已批准；19 条；当前 pre 已部署 | 当前 Dashboard 必需，每个目标 Prometheus 环境加载一次 |
| `helix-runtime-performance-pre` | 可选性能分析组 | 仅在 pre/压测环境且对应原始指标存在时部署 |
| `messagev3-business-reliability` | 待重新评审 | 暂不迁移；窗口、分母与无流量语义尚需统一 |

`messagev3-business-reliability` 当前不能盲目迁移，至少还有三项合同债务：

1. 部分比率使用 `1h` 或 `6h`，与全链路 Dashboard 默认 `5m` 观察窗口不一致。
2. recipient enqueue 分母只覆盖 `enqueued|slow_consumer`，尚未明确
   `connection_missing`、`delivery_probe_rejected` 是否进入失败分母。
3. 无流量、样本不足和遥测出口中断的显示语义尚未完全统一，可能产生假绿或除零空值。

新环境只应从已批准 Group 生成完整 `PrometheusRule`：

```yaml
apiVersion: monitoring.coreos.com/v1
kind: PrometheusRule
metadata:
  name: messagev3-fullchain-observability
  namespace: monitoring
  labels:
    release: prometheus-stack
    app.kubernetes.io/part-of: im
spec:
  groups:
    # 仅复制 recording-rules.yaml 中已批准的
    # messagev3-fullchain-observability group。
```

不得直接把仓库中三个 Group 全部包装后应用。Rule 变更仍须先执行指标语义 Review，得到
`REVIEW: APPROVED` 后才进入环境变更。

## 5. 环境拓扑矩阵

| 项目 | 本地 Docker | 当前 pre Kubernetes | prod 迁移要求 |
|---|---|---|---|
| Helix OTLP gRPC | `otel-collector:4317` | `opentelemetry-collector.monitoring:4317` | TLS、鉴权、DNS 与网络策略明确 |
| Helix OTLP HTTP | `otel-collector:4318` | `opentelemetry-collector.monitoring:4318` | 与客户端出口策略一致 |
| Collector Prometheus 出口 | `:9464` | Service `:8889` | 固定命名端口并由 ServiceMonitor 引用 |
| GoServer | `host.docker.internal:8066/metrics` | 由 Go Service/ServiceMonitor 决定 | 独立 Target，不能经 Trace 推导代替 |
| Collector 抓取 | static scrape | ServiceMonitor，`/metrics`，30s | Service selector 与 release label 对齐 |
| Recording Rule | 按本地 Prometheus 配置加载 | Group 1 已部署 | 已批准 Group 每环境一次 |
| Grafana | 本地导入 JSON | Dashboard UID `msgv3-obsv-v1` | Provisioning/GitOps、数据源 UID 对齐 |

当前 pre 已核实：

- namespace 为 `monitoring`；Collector Service 暴露 `4317`、`4318`、`8889`。
- `ServiceMonitor/opentelemetry-collector` 每 30 秒抓取命名端口 `metrics` 的 `/metrics`。
- `PrometheusRule/messagev3-fullchain-observability` 已加载同名 Group，共 19 条规则。
- Prometheus 当前显示 `cses-im-server`、`cses-im-server-local-metrics` 和
  `opentelemetry-collector` Target 均为 `up`；19 条必需 Rule 均为 `health=ok`。
- 当前查询 `im_http_server_requests_total` 有序列，但 `im_message_client_terminal_total` 和
  `im_tick_stage_duration_seconds_count` 仍无结果。因此基础设施已接通，Helix/客户端真实业务
  样本尚未完成，不能把当前 pre 判为 Dashboard 全绿。

## 6. 每个新环境还要迁移什么

Recording Rules 只是其中一层。完整迁移至少包含以下项目：

1. **制品与版本**：GoServer 镜像 digest、Helix 版本、客户端版本、Dashboard version 与 Rule
   版本必须可追溯；资源标签应含 `service.name`、`service.version`、`deployment.environment`。
2. **OTel Collector**：OTLP receiver、`memory_limiter`、`batch`、Prometheus exporter、健康检查、
   自观测指标、资源上限和失败队列策略。
3. **服务发现**：Collector Service/ServiceMonitor；GoServer Service/ServiceMonitor；selector、
   namespaceSelector、端口名与 Prometheus Operator 的 `release` label 必须匹配。
4. **Prometheus**：scrape/evaluation interval、已批准 Rule、TSDB 保留期、磁盘容量、时间同步和
   Target/Rule 自监控。
5. **Grafana**：Prometheus datasource UID、Dashboard provisioning、变量、时间窗口、刷新频率、
   权限与 `No data` 行为。
6. **告警**：Collector/Go Target Down、遥测丢弃、Rule Evaluation Error、错误率和 SLO 告警；
   Alertmanager 路由需要单独迁移，Recording Rule 本身不会发送告警。
7. **网络与安全**：OTLP 端口可达、NetworkPolicy、防火墙、TLS/鉴权、Grafana/Trace 下钻授权、
   数据脱敏和 TTL。
8. **真实流量**：至少一台对应版本客户端持续在线并触发 send、echo、receive、apply、view update、
   sync/gap 与 reconnect；没有真实流量时 Dashboard 不可能全绿。
9. **容量保护**：禁止高基数 ID；验证 Collector 队列、丢弃计数、Prometheus series 增长和
   Dashboard 慢查询。

## 7. Kubernetes 迁移顺序

### 7.1 迁移前盘点

```bash
kubectl config current-context
kubectl get namespace monitoring
kubectl -n monitoring get deploy,svc,servicemonitor,prometheusrule
kubectl -n monitoring get pods -o wide
```

先记录目标环境、资源 owner、现有 selector、镜像 digest 和回滚版本。禁止在未确认 context 时
执行 apply。

### 7.2 Collector 与抓取入口

1. 部署或升级 Collector ConfigMap/Deployment。
2. 验证 `4317/4318` 只用于 OTLP 接收，Prometheus exporter 使用独立命名端口。
3. 创建或核对 Collector Service 与 ServiceMonitor。
4. 创建或核对 GoServer Service 与 ServiceMonitor；Go `/metrics` 直接抓取，不必先转 OTLP。

### 7.3 Recording Rules

1. 从仓库只提取已批准的 Group。
2. 包装为目标 namespace 下的完整 `PrometheusRule` CR。
3. 先用目标集群的 schema/server dry-run 验证，再应用。
4. 检查 Prometheus Operator 是否接纳资源及每条规则的 Health。

```bash
kubectl apply --dry-run=server -f /绝对路径/已批准的-prometheusrule.yaml
kubectl apply -f /绝对路径/已批准的-prometheusrule.yaml
kubectl -n monitoring get prometheusrule messagev3-fullchain-observability -o yaml
```

### 7.4 Grafana

导入或 provision [`messagev3-fullchain-observability.json`](../../perf/grafana/messagev3-fullchain-observability.json)，
保持 UID `msgv3-obsv-v1`，并确认 datasource UID 为目标环境真实的 Prometheus 数据源。旧 Dashboard
不覆盖；新旧版本并存至验收完成。

## 8. 分层验证

以下证据不能互相替代，必须逐层完成：

| 层级 | 验证内容 | 通过标准 |
|---|---|---|
| 生产代码 | recorder 已接入真实终点 | focused test/build 通过；不等于运行样本 |
| Exporter | Go `/metrics`、Collector exporter 可访问 | HTTP 成功且目标 family 存在 |
| Prometheus Target | Go 与 Collector 都被抓取 | `up == 1`，无持续 scrape error |
| 原始样本 | 本次版本产生近期 timestamp | metadata 有名字，query 有非空近期样本 |
| Recording Rule | Prometheus 完成评估 | 每条 rule `health=ok` 且 `lastError` 为空 |
| 规范指标 | `im_*` 查询有近期样本 | 与对应原始序列口径对账 |
| 真实业务链 | 实际消息/同步/重连走完整路径 | 关键终点与关联 Trace 可对账 |
| Grafana | 面板查询、变量和状态正确 | 无查询错误；有流量显示真实值，无流量显示 No data |

常用只读检查：

```bash
kubectl -n monitoring get servicemonitor opentelemetry-collector -o yaml
kubectl -n monitoring get prometheusrule messagev3-fullchain-observability -o yaml
kubectl -n monitoring get pods -l app.kubernetes.io/name=opentelemetry-collector
```

Prometheus API 需要通过目标环境允许的端口转发或网关访问，重点查询：

```text
/api/v1/targets
/api/v1/metadata
/api/v1/rules?type=record
/api/v1/query?query=up
/api/v1/query?query=im_message_client_terminal_total
```

## 9. 验收条件

环境迁移完成必须同时满足：

- GoServer 与 Collector Target 均为 `up=1`。
- Go、Helix 和客户端终点指标都有本次发布版本的近期真实样本。
- 19 条必需 Recording Rule 全部健康，规范 `im_*` 指标可查询。
- send → Go persist → Helix event → client received → client applied → view updated 可对账。
- live、sync replay、offline recovery、gap pending/real gap、WS disconnect/reconnect 都至少有一次真实证据。
- Dashboard 无查询错误；有流量的目标面板有数据，无流量或未接入的面板诚实显示 `No data`。
- 遥测不可达、队列满或上报失败不会阻塞 Go 业务、Helix Tick 或 Angular UI 线程。
- Channel/User 精确诊断受授权、TTL、脱敏与容量上限约束，不向 Prometheus 注入高基数标签。

“Dashboard 全绿”只能作为最终展示结果，不能代替上述各层证据。`up=1` 也只表示抓取目标健康，
不表示消息送达成功。

## 10. 回滚

迁移前保留 Collector ConfigMap、Deployment、ServiceMonitor、PrometheusRule 和 Dashboard 的上一
版本。回滚时按相反顺序恢复观测资源，不回滚或阻塞业务链：

1. 恢复上一版 Dashboard/Rule，避免新查询持续报错。
2. 恢复上一版 ServiceMonitor 和 Collector 配置。
3. 核对 GoServer 与客户端业务仍可运行。
4. 记录被移除规则的规范序列会随 Prometheus 保留期自然过期，不做破坏性 TSDB 清理。

观测失败的默认降级是丢弃观测并告警，不是阻塞消息发送、持久化、同步或界面渲染。
