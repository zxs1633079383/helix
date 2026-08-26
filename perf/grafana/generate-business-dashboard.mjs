#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const datasource = { type: "prometheus", uid: "prometheus" };
let panelId = 1;
let y = 0;

// row 创建一个中文业务分组，避免技术阶段与业务 SLO 混在同一视图。
function row(title) {
  return { id: panelId++, title, type: "row", collapsed: false, gridPos: { h: 1, w: 24, x: 0, y: y++ }, panels: [] };
}

// statPanel 创建成功率或异常率单值面板，零分母保持“无数据”。
function statPanel(title, expr, description, invert = false) {
  const good = invert ? 0.000001 : 0.999;
  return {
    id: panelId++, title, description, type: "stat", datasource,
    gridPos: { h: 6, w: 8, x: 0, y: y++ },
    fieldConfig: {
      defaults: {
        unit: "percentunit", noValue: "无真实终态数据",
        thresholds: { mode: "absolute", steps: invert
          ? [{ color: "green", value: null }, { color: "red", value: good }]
          : [{ color: "red", value: null }, { color: "green", value: good }] },
      }, overrides: [],
    },
    options: { reduceOptions: { calcs: ["lastNotNull"], fields: "", values: false }, colorMode: "background", graphMode: "area", justifyMode: "auto", textMode: "auto" },
    targets: [{ refId: "A", expr, instant: true, legendFormat: title }],
  };
}

// throughputPanel 展示真实事务成功量与排除幂等命中的物理新增消息量，避免混淆两个口径。
function throughputPanel() {
  return {
    id: panelId++, title: "远端消息落库量 / 秒", type: "timeseries", datasource,
    description: "新增消息/秒只统计 outcome=inserted；幂等命中单列，不冒充物理新增。事务成功/秒用于观察旧版本或全量提交吞吐。",
    gridPos: { h: 8, w: 24, x: 0, y: y++ },
    fieldConfig: { defaults: { unit: "ops", noValue: "无采集样本", min: 0, color: { mode: "palette-classic" } }, overrides: [] },
    options: { legend: { displayMode: "table", placement: "bottom", calcs: ["lastNotNull", "max"] }, tooltip: { mode: "multi", sort: "desc" } },
    targets: [
      { refId: "A", expr: 'im:message_remote_persist_inserted_per_second{job=~"$go_job",environment=~"$go_environment"}', legendFormat: "{{environment}} 真实新增消息/秒" },
      { refId: "B", expr: 'im:message_remote_persist_deduplicated_per_second{job=~"$go_job",environment=~"$go_environment"}', legendFormat: "{{environment}} 幂等命中/秒" },
      { refId: "C", expr: 'sum by (job, environment) (rate(im_message_persist_terminal_total{job=~"$go_job",environment=~"$go_environment",result="success"}[5m]) and on(job, environment, instance) (up == 1))', legendFormat: "{{environment}} 成功事务/秒" },
    ],
  };
}

// latencyPanel 同屏展示 Go 远端提交与 Helix 权威投影的 P50/P95/P99，不伪造跨进程逐消息 join。
function latencyPanel() {
  return {
    id: panelId++, title: "消息关键阶段延迟 P50 / P95 / P99", type: "timeseries", datasource,
    description: "Go 远端事务与 Helix command→projection 是两个独立阶段；客户端 ACK 未接通前不称为完整端到端延迟。",
    gridPos: { h: 9, w: 24, x: 0, y: y++ },
    fieldConfig: { defaults: { unit: "s", noValue: "无数据", color: { mode: "palette-classic" } }, overrides: [] },
    options: { legend: { displayMode: "table", placement: "bottom", calcs: ["lastNotNull", "max"] }, tooltip: { mode: "multi", sort: "desc" } },
    targets: [
      { refId: "A", expr: 'im:message_remote_persist_duration:p50_5m{job=~"$go_job",environment=~"$go_environment"}', legendFormat: "Go {{environment}} 落库 P50" },
      { refId: "B", expr: 'im:message_remote_persist_duration:p95_5m{job=~"$go_job",environment=~"$go_environment"}', legendFormat: "Go {{environment}} 落库 P95" },
      { refId: "C", expr: 'im:message_remote_persist_duration:p99_5m{job=~"$go_job",environment=~"$go_environment"}', legendFormat: "Go {{environment}} 落库 P99" },
      { refId: "D", expr: 'helix:message_projection_duration:p50_5m{job=~"$helix_job",deployment_environment=~"$helix_environment"}', legendFormat: "Helix {{deployment_environment}} 投影 P50 {{platform}}" },
      { refId: "E", expr: 'helix:message_projection_duration:p95_5m{job=~"$helix_job",deployment_environment=~"$helix_environment"}', legendFormat: "Helix {{deployment_environment}} 投影 P95 {{platform}}" },
      { refId: "F", expr: 'helix:message_projection_duration:p99_5m{job=~"$helix_job",deployment_environment=~"$helix_environment"}', legendFormat: "Helix {{deployment_environment}} 投影 P99 {{platform}}" },
    ],
  };
}

const panels = [
  row("00 十二项核心业务指标"),
  statPanel("消息远端落库成功率", 'im:message_persist_success_ratio{job=~"$go_job",environment=~"$go_environment"}', "Go 一小时内事务 commit 成功 / 全部事务终态。"),
  statPanel("Outbox 最终投递成功率", 'im:delivery_outbox_success_ratio{job=~"$go_job",environment=~"$go_environment"}', "一小时内 delivered / created；重试跨窗口时需结合 backlog 与 lag 解读。"),
  statPanel("每接收人入队成功率", 'im:recipient_enqueue_success_ratio{job=~"$go_job",environment=~"$go_environment"}', "enqueued / (enqueued + slow_consumer)，只统计通过订阅过滤的目标连接。"),
  statPanel("客户端真实 ACK 送达率", 'helix:im_client_ack_success_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "Helix 本地权威投影后发起 ACK，以 HTTP 2xx 且 Go status=SUCCESS 作为真实送达终态。"),
  statPanel("Go ACK 接收成功率", 'im:client_ack_success_ratio{job=~"$go_job",environment=~"$go_environment"}', "Go 对已到达 ACK 执行会话、消息可见性与 Redis 幂等校验后的业务成功率；与客户端送达率分开观察。"),
  statPanel("Helix 本地权威投影成功率", 'helix:im_projection_success_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "仅统计 PersistOk/read-back 后的稳定 MessageV3 terminal event。"),
  statPanel("同步会话成功率", 'helix:im_sync_session_success_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "每轮 channel/startup/recovery 会话只记录一个终态。"),
  statPanel("同步异常会话率", 'helix:im_sync_anomaly_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "TooLong 等异常终态 / 全部同步会话终态。", true),
  statPanel("离线 / 断线恢复成功率", 'helix:im_recovery_success_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "offline recovery、gap repair、channel hydration 的稳定终态。"),
  statPanel("未读对账不一致率", 'helix:im_unread_mismatch_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "同 scope 的远端与本地未读绝对值对账终态。", true),
  statPanel("消息丢失 / 重复 / 乱序率", 'helix:im_message_correctness_error_ratio{job=~"$helix_job",deployment_environment=~"$helix_environment"}', "只统计 Runtime 真实检测点；HarnessOnly 压测结果不进入。", true),
  statPanel("业务 Command 终态成功率", 'sum by (job, environment, operation) (increase(im_business_command_terminal_total{job=~"$go_job",environment=~"$go_environment",result="success"}[1h]) and on(job, environment, instance) (up == 1)) / sum by (job, environment, operation) (increase(im_business_command_terminal_total{job=~"$go_job",environment=~"$go_environment"}[1h]) and on(job, environment, instance) (up == 1))', "Go 业务 Command 的真实 terminal success / 全部 terminal；按 operation 独立展示，不包含 accepted/rejected admission。"),
  throughputPanel(),
  latencyPanel(),
];

let rowY = 0;
let x = 0;
for (const panel of panels) {
  if (panel.type === "row") {
    panel.gridPos = { h: 1, w: 24, x: 0, y: rowY++ };
    x = 0;
    continue;
  }
  if (panel.type === "timeseries") {
    panel.gridPos = { h: 9, w: 24, x: 0, y: rowY };
    rowY += 9;
    x = 0;
    continue;
  }
  panel.gridPos = { h: 6, w: 8, x, y: rowY };
  x += 8;
  if (x === 24) {
    x = 0;
    rowY += 6;
  }
}

const dashboard = {
  annotations: { list: [] }, editable: true, graphTooltip: 1, id: null,
  links: [], panels, refresh: "30s", schemaVersion: 39, tags: ["MessageV3", "业务可靠性"],
  templating: { list: [
    { name: "go_job", label: "GoServer job", type: "query", datasource, query: { query: "label_values(im_send_inflight, job)", refId: "go-job" }, current: { selected: false, text: ".*", value: ".*" }, includeAll: true, allValue: ".*", refresh: 1 },
    { name: "go_environment", label: "Go 环境", type: "query", datasource, query: { query: "label_values(im_send_inflight, environment)", refId: "go-environment" }, current: { selected: true, text: "dev", value: "dev" }, includeAll: true, allValue: ".*", refresh: 1 },
    { name: "helix_job", label: "Helix job", type: "query", datasource, query: { query: "label_values(otel_helix_im_projection_terminal_total, job)", refId: "helix-job" }, current: { selected: false, text: ".*", value: ".*" }, includeAll: true, allValue: ".*", refresh: 1 },
    { name: "helix_environment", label: "Helix 环境", type: "query", datasource, query: { query: "label_values(otel_helix_im_projection_terminal_total, deployment_environment)", refId: "helix-environment" }, current: { selected: true, text: "dev", value: "dev" }, includeAll: true, allValue: ".*", refresh: 1 },
  ] },
  time: { from: "now-6h", to: "now" }, timezone: "browser",
  title: "MessageV3 业务可靠性", uid: "messagev3-business-reliability", version: 1,
};

fs.writeFileSync(path.join(dir, "messagev3-business-reliability.json"), `${JSON.stringify(dashboard, null, 2)}\n`);
console.log(`generated MessageV3 business dashboard: panels=${panels.length}`);
