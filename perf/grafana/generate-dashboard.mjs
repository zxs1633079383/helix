#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const contract = JSON.parse(fs.readFileSync(path.join(dir, "dashboard.contract.json"), "utf8"));
const datasource = { type: "prometheus", uid: contract.datasourceUid };
const selector = 'service_name=~"$service",platform=~"$platform",scenario=~"$scenario",channel_shape=~"$channel_shape",stage=~"$stage",operation=~"$operation"';
let panelId = 1;
let y = 0;

const zhNames = {
  helix_operations_total: "吞吐（QPS / RPS）",
  helix_errors_total: "错误数、速率与错误率",
  helix_core_step_duration_seconds: "Core step 延迟",
  helix_tick_queue_wait_seconds: "Tick 队列等待",
  helix_tick_queue_depth: "Tick 队列深度",
  helix_tick_queue_capacity: "Tick 队列容量",
  helix_effects_per_tick: "每 Tick Effect 数",
  helix_effect_bytes_per_tick: "每 Tick Effect 字节",
  helix_effect_amplification_ratio: "Effect 放大比",
  helix_http_queue_wait_seconds: "HTTP 队列等待",
  helix_http_request_duration_seconds: "HTTP 请求延迟",
  helix_http_inflight: "HTTP 并发请求",
  helix_persist_queue_wait_seconds: "持久化队列等待",
  helix_storage_tx_duration_seconds: "SQLite 事务延迟",
  helix_storage_rows_total: "Storage 行处理吞吐",
  helix_port_roundtrip_seconds: "PortReply 往返延迟",
  helix_ws_ingress_to_effect_seconds: "WS 入站到 Effect 延迟",
  helix_ws_ingress_to_event_seconds: "WS 入站到 Event 延迟",
  helix_command_to_projection_seconds: "Command 到 Projection 延迟",
  helix_event_lagged_total: "Event lagged",
  helix_event_no_receiver_total: "Event 无接收者",
  helix_event_batch_size: "Event 批大小",
  helix_ffi_batch_events: "FFI 批事件数",
  helix_ffi_batch_bytes: "FFI 批字节数",
  helix_ffi_callback_duration_seconds: "FFI 回调延迟",
  helix_ffi_busy_total: "FFI busy",
  helix_alloc_count_per_op: "每操作分配次数",
  helix_alloc_bytes_per_op: "每操作分配字节",
  helix_process_resident_memory_bytes: "进程常驻内存",
  helix_shutdown_drain_seconds: "关闭排空延迟",
  helix_metrics_queue_depth: "指标队列深度",
  helix_metrics_queue_capacity: "指标队列容量",
  helix_metrics_dropped_total: "指标丢弃",
  helix_metrics_export_batch_size: "指标导出批大小",
  helix_metrics_export_duration_seconds: "指标导出延迟",
  helix_metrics_export_errors_total: "指标导出错误",
  helix_metrics_exporter_state: "指标导出器状态",
  helix_metrics_last_success_age_seconds: "距最近成功导出秒数"
};

function promName(name) {
  return `otel_${name}`;
}

function basePanel(title, type = "timeseries") {
  return {
    id: panelId++,
    title,
    type,
    datasource,
    gridPos: { h: 8, w: 12, x: 0, y: y++ },
    fieldConfig: {
      defaults: { color: { mode: "palette-classic" }, custom: { drawStyle: "line", lineWidth: 1, fillOpacity: 8 }, noValue: "无数据" },
      overrides: []
    },
    options: { legend: { displayMode: "table", placement: "bottom", calcs: ["lastNotNull", "mean", "max"] }, tooltip: { mode: "multi", sort: "desc" } },
    targets: []
  };
}

function histogramPanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  const grouped = `sum by (le) (last_over_time(${metricName}_bucket{${selector}}[$rate_window]))`;
  p.targets = [
    ["P50", "0.50"], ["P90", "0.90"], ["P95", "0.95"], ["P99", "0.99"]
  ].map(([legend, q], index) => ({ refId: String.fromCharCode(65 + index), expr: `histogram_quantile(${q}, ${grouped})`, legendFormat: legend }));
  p.targets.push({
    refId: "E",
    expr: `sum(last_over_time(${metricName}_sum{${selector}}[$rate_window])) / clamp_min(sum(last_over_time(${metricName}_count{${selector}}[$rate_window])), 0.000001)`,
    legendFormat: "AVG"
  });
  if (metric.endsWith("_seconds")) p.fieldConfig.defaults.unit = "s";
  else if (metric.includes("bytes")) p.fieldConfig.defaults.unit = "bytes";
  else p.fieldConfig.defaults.unit = "short";
  return p;
}

function counterPanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  p.targets = [
    { refId: "A", expr: `sum by (stage, operation, status) (rate(${metricName}{${selector}}[$rate_window]))`, legendFormat: "RATE {{stage}} / {{operation}} / {{status}}" },
    { refId: "B", expr: `sum by (stage, operation, status) (increase(${metricName}{${selector}}[$__range]))`, legendFormat: "INCREASE {{stage}} / {{operation}} / {{status}}" },
    { refId: "C", expr: `sum by (stage, operation, status) (last_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "TOTAL {{stage}} / {{operation}} / {{status}}" }
  ];
  if (metric === "helix_errors_total") {
    p.targets.push({
      refId: "D",
      expr: `sum(rate(otel_helix_operations_total{${selector},status="error"}[$rate_window])) / clamp_min(sum(rate(otel_helix_operations_total{${selector}}[$rate_window])), 0.000001)`,
      legendFormat: "ERROR_RATIO"
    });
  }
  p.fieldConfig.defaults.unit = metric === "helix_operations_total" ? "ops" : "short";
  return p;
}

function gaugePanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  p.targets = [
    { refId: "A", expr: `max by (stage, operation) (${metricName}{${selector}})`, legendFormat: "CURRENT {{stage}} / {{operation}}" },
    { refId: "B", expr: `avg(avg_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "AVG" },
    { refId: "C", expr: `max(max_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "MAX" }
  ];
  p.fieldConfig.defaults.unit = metric.includes("bytes") ? "bytes" : metric.endsWith("_seconds") ? "s" : "short";
  return p;
}

function row(title) {
  return { id: panelId++, title, type: "row", collapsed: false, gridPos: { h: 1, w: 24, x: 0, y: y++ }, panels: [] };
}

function overviewPanel(title, expr, legend, unit = "ops") {
  const p = basePanel(title);
  p.targets = [{ refId: "A", expr, legendFormat: legend }];
  p.fieldConfig.defaults.unit = unit;
  return p;
}

const panels = [];
panels.push(row("00 总览"));
const throughput = "otel_helix_run_throughput_per_second";
panels.push(overviewPanel("HTTP RPS", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="http",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("WebSocket 入站/出站 RPS", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="ws",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("Storage QPS", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="storage",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("Event RPS", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="event",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("错误总数", `sum by (stage, operation, error_kind) (last_over_time(otel_helix_errors_total{${selector}}[$rate_window]))`, "{{stage}} / {{operation}} / {{error_kind}}", "short"));
panels.push(overviewPanel("错误速率", `sum(last_over_time(${throughput}{${selector},stage="core",status="error"}[$rate_window]))`, "ERROR_RATE"));
panels.push(overviewPanel("错误率", `sum(last_over_time(${throughput}{${selector},stage="core",status="error"}[$rate_window])) / clamp_min(sum(last_over_time(${throughput}{${selector},stage="core"}[$rate_window])), 0.000001)`, "ERROR_RATIO", "percentunit"));

for (const rowName of [...new Set(contract.requiredMetrics.map((m) => m.row))]) {
  if (rowName !== "00 总览") panels.push(row(rowName));
  for (const metric of contract.requiredMetrics.filter((m) => m.row === rowName)) {
    const title = zhNames[metric.name] ?? metric.name;
    if (metric.kind === "histogram") panels.push(histogramPanel(metric.name, title));
    if (metric.kind === "counter") panels.push(counterPanel(metric.name, title));
    if (metric.kind === "gauge") panels.push(gaugePanel(metric.name, title));
  }
}

panels.push(row("100 正确性与 Canary"));
for (const metric of contract.correctnessMetrics) {
  const p = counterPanel(metric, `正确性：${metric.replace(/^helix_|_total$/g, "")}`);
  p.fieldConfig.defaults.thresholds = { mode: "absolute", steps: [{ color: "green", value: null }, { color: "red", value: 0.000001 }] };
  panels.push(p);
}
const canary = basePanel("遥测 Canary", "stat");
canary.targets = [{ refId: "A", expr: `sum(${promName(contract.canaryMetric)}{service_name=~"$service"})`, legendFormat: "CANARY" }];
canary.fieldConfig.defaults.thresholds = { mode: "absolute", steps: [{ color: "red", value: null }, { color: "green", value: 1 }] };
canary.options = { reduceOptions: { calcs: ["lastNotNull"], fields: "", values: false }, colorMode: "background", graphMode: "area", justifyMode: "auto", textMode: "auto" };
panels.push(canary);

let rowY = 0;
let x = 0;
for (const panel of panels) {
  if (panel.type === "row") {
    panel.gridPos = { h: 1, w: 24, x: 0, y: rowY++ };
    x = 0;
  } else {
    panel.gridPos = { h: panel.type === "stat" ? 6 : 8, w: 12, x, y: rowY };
    x = x === 0 ? 12 : 0;
    if (x === 0) rowY += panel.gridPos.h;
  }
}

const dashboard = {
  annotations: { list: [{ builtIn: 1, datasource: { type: "grafana", uid: "-- Grafana --" }, enable: true, hide: true, iconColor: "rgba(0, 211, 255, 1)", name: "Annotations & Alerts", type: "dashboard" }] },
  editable: true,
  fiscalYearStartMonth: 0,
  graphTooltip: 1,
  id: null,
  links: [],
  liveNow: false,
  panels,
  refresh: "30s",
  schemaVersion: 41,
  tags: ["helix", "performance", "otel", "pre"],
  templating: {
    list: [
      { name: "datasource", label: "数据源", type: "datasource", query: "prometheus", current: { selected: true, text: "Prometheus", value: contract.datasourceUid }, options: [], refresh: 1 },
      { name: "service", label: "服务", type: "query", datasource, query: { query: "label_values(otel_helix_operations_total, service_name)", refId: "service" }, includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, refresh: 1 },
      { name: "platform", label: "平台", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service"}, platform)', refId: "platform" }, includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, refresh: 1 },
      { name: "scenario", label: "场景", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform"}, scenario)', refId: "scenario" }, includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, refresh: 1 },
      { name: "channel_shape", label: "频道形态", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform",scenario=~"$scenario"}, channel_shape)', refId: "channel_shape" }, includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, refresh: 1 },
      { name: "stage", label: "阶段", type: "custom", query: "core,effect,http,ws,storage,port_reply,event,ffi,telemetry", includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, options: [] },
      { name: "operation", label: "操作", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform",scenario=~"$scenario",channel_shape=~"$channel_shape",stage=~"$stage"}, operation)', refId: "operation" }, includeAll: true, allValue: ".*", current: { text: "All", value: "$__all" }, refresh: 1 },
      { name: "rate_window", label: "统计窗口", type: "custom", query: "2m,5m,15m,30m", current: { selected: true, text: "5m", value: "5m" }, options: [{ selected: false, text: "2m", value: "2m" }, { selected: true, text: "5m", value: "5m" }, { selected: false, text: "15m", value: "15m" }, { selected: false, text: "30m", value: "30m" }] }
    ]
  },
  time: { from: "now-30m", to: "now" },
  timepicker: {},
  timezone: "browser",
  title: contract.title,
  uid: contract.uid,
  version: 1,
  weekStart: ""
};

fs.writeFileSync(path.join(dir, "helix-runtime-performance-pre.json"), `${JSON.stringify(dashboard, null, 2)}\n`);
