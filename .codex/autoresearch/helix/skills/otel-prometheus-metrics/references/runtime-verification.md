# OTel/Prometheus 运行验证参考

## 1. 证据等级

| 等级 | 需要的证据 | 能证明什么 | 不能证明什么 |
|---|---|---|---|
| `FOCUSED` | registry/producer/worker 测试 | 本地分类、标签、队列、重试和熔断 | Collector 收到真实业务样本 |
| `CONFIG_STATIC` | YAML、Collector pipeline、Prom scrape、dashboard contract | 配置与生成物一致 | 进程实际导出或样本存在 |
| `EXPORT` | worker 日志、`HELIX_METRICS_EXPORT_OK`、exporter state | exporter 运行并完成一次导出 | Prometheus 已抓到目标业务序列 |
| `COLLECTOR_SCRAPE` | `:9464/metrics` 出现目标名 | Collector 的 Prometheus exporter 有目标数据 | Prometheus 查询标签和时间窗口正确 |
| `PROM_QUERY` | `/api/v1/metadata` 和 `/api/v1/query` | Prometheus 已抓到目标 metric，当前有非空新鲜 sample | 业务事件一定是本轮触发的 |
| `REAL_BUSINESS` | 同一窗口的真实业务触发、service/resource label 和 query | 真实业务链路到 Prometheus 闭合 | 其他平台或其他 commit 的运行态 |

任何等级都不能用 HTTP 200、健康检查、静态注册或 synthetic telemetry canary 代替更高等级。

## 2. 先做只读预检

```bash
python3 ~/.codex/scripts/log_preflight.py
git status --short --branch
curl -fsS http://127.0.0.1:9464/metrics
curl -fsS http://127.0.0.1:9090/-/ready
```

如果本地端口或 Docker 不存在，报告 `COLLECTOR_UNAVAILABLE`，不要修改业务代码来“修绿”。

## 3. Prometheus API 验证

使用脚本：

```bash
python3 scripts/verify_prometheus_metric.py \
  --prom-url http://127.0.0.1:9090 \
  --metric otel_helix_im_sync_timeout_total \
  --selector 'service_name="helix-host-cli",deployment_environment="local"' \
  --max-age-seconds 120
```

脚本必须同时满足：

1. `/api/v1/metadata?metric=...` 返回 `status=success` 且有 metadata；
2. `/api/v1/query?query=...` 返回 `status=success`；
3. instant vector 非空，且至少一个样本时间在 `max-age-seconds` 内；
4. 若指定 `--min-value`，至少一个数值样本达到阈值；
5. 输出不回显完整业务 payload，退出码可供 CI/人工脚本判断。

查询结果为空时区分：

- `PROM_METADATA_MISSING`：Prometheus 还没有该名称的 metadata；
- `PROM_QUERY_EMPTY`：metadata 有，但当前 selector 无 series；
- `STALE_SAMPLE`：有 series，但最近样本超出窗口；
- `CONFIG_ONLY`：只有配置或 worker 启动证据。

## 4. Collector 与真实事件

先在 Collector exporter 端点确认目标名，再触发已批准的真实业务事件。事件触发前记录
commit、service name、deployment environment、selector、开始时间和结束时间；触发后在
同一窗口重新执行 Collector scrape 与 Prometheus query。

不要用 `perf/telemetry/telemetry.sh` 的 canary POST 来证明新业务指标；该脚本只适合验证
Collector/Prometheus 基础设施通路。不要向生产 Prometheus 写入或删除样本。

## 5. 结果格式

建议保留如下证据对象：

```json
{
  "level": "PROM_QUERY",
  "metric": "otel_helix_im_sync_timeout_total",
  "selector": "service_name=\"helix-host-cli\"",
  "status": "PASS",
  "metadata_count": 1,
  "sample_count": 1,
  "freshest_age_seconds": 8.4,
  "proves": "Prometheus 在当前窗口返回目标指标样本",
  "does_not_prove": "其他平台或其他 commit 已采集"
}
```

报告中单列 `unverified` 和 `risks`；不能把 `PARTIAL` 自动升级为 `PASS`。
