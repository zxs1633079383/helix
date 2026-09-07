# Metrics 文档与看板入口

| 文档 | 用途 |
|---|---|
| [Prometheus 指标说明](Prometheus指标说明.md) | 注册表、producer 与 Runtime/Planned 等边界 |
| [客户端当前真实暴露指标](Angular客户端当前真实暴露指标-2026-09-07.md) | 2026-09-07 PRE 的 71 个实测 Helix family 与缺失业务样本 |
| [客户端指标快照 JSON](客户端指标快照-2026-09-07.json) | 可机器对账的名称/类型/版本边界，不含用户ID或消息正文 |
| [用户与群聊消息可靠性设计](用户与群聊消息可靠性观测设计.md) | 登录少群、接收/落库、离线同步、Gap 区间、每群 seq 与未来采集方案 |
| [观测环境迁移手册](MessageV3全链路观测环境迁移手册.md) | Collector、Prometheus、Grafana 与部署责任 |

## 本次独立 Dashboard

[客户端与群聊观测 JSON](../../perf/grafana/client-channel-observability.json) 由 [生成器](../../perf/grafana/generate-client-channel-dashboard.mjs) 与 [contract](../../perf/grafana/client-channel-dashboard.contract.json) 生成；不要手改生成 JSON，也不覆盖现有两个业务看板。

```bash
node perf/grafana/generate-client-channel-dashboard.mjs
node perf/grafana/client-channel-dashboard-check.mjs
node perf/grafana/client-channel-dashboard-check.mjs --live
```

`--live` 只读查询 PRE Prometheus/Loki，校验全局与指定用户的插值/语法/不回退全局。日志空数据不算producer验收。可通过 `HELIX_PRE_PROMETHEUS_URL` 与 `HELIX_DIAGNOSTIC_LOKI_URL` 覆盖目标。

已发布：[打开客户端与群聊观测看板](http://192.168.6.66:30300/d/helix-client-channel-v1/fd6d377)，位于 Helix 文件夹，版本1。默认绑定现有 `Prometheus`（UID `prometheus`）和 `loki-pre`（UID `ef65piv34jegwd`），未修改数据源配置。Grafana `12.2.1`、PRE Prometheus 和 Loki `2.9.3` 已通过认证数据源代理查询。其他环境可上传 JSON 导入，并在顶部数据源变量中选择对应实例。

- 默认 user_id/channel_id/login_attempt_id/device_session_id 留空，表示当前授权范围全局。
- 在顶部输入准确 ID，或在登录/诊断表格的 ID 单元格点击下钻。没有虚构“自动列出所有用户”的变量：Loki 未索引的 JSON user_id 不能通过 label_values 枚举。
- 环境、源服务和租户需按部署设置。默认 `pre`、`cses-client-pre`；租户为空只供已获全局权限的运维。
- 技术面板只适用于全局；任一身份筛选非空时显示无身份维度，不借全局值冒充用户值。
- 诊断面板需要设计中 `service_name=helix-im-diagnostics` 的 JSON 事件，当前未接入；表现为空值而非0%/100%。原生OTLP Logs接入须处理Loki版本与pipeline前置。
- seq字段是字符串；群检查点表仅展示窗口内最后观察，不代表完整当前数据库。准确全量每群状态依赖设计中的授权 latest-state 查询服务。
- 默认30分钟，日志每面板最多1000条；全局观察更长窗口可能被截断，先选租户/用户/登录，不能把表中缺失当作群不存在。
- 两路 Span-derived 指标不可相加为业务量；当前还缺deployment_environment，严格环境过滤时这4个面板为空；没有实际业务终态数据时看板不提供已验证的送达率。

## 验证证据

本轮验收结果记录在[看板交付与验证](看板交付与验证-2026-09-07.md)。未来接入producer、Collector及真实双用户业务后需要重新验收，不能沿用本轮静态/查询语法通过作为业务闭环证据。
