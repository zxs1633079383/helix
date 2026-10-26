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
  helix_operations_total: "吞吐（每秒操作数）",
  helix_errors_total: "错误数、速率与错误率",
  helix_command_accepted_total: "命令接纳总数",
  helix_command_rejected_total: "命令拒绝总数",
  helix_command_admission_duration_seconds: "命令接入耗时",
  helix_core_step_duration_seconds: "核心步进耗时",
  helix_tick_queue_wait_seconds: "驱动输入队列等待耗时",
  helix_tick_queue_depth: "驱动输入队列深度",
  helix_tick_queue_capacity: "驱动输入队列容量",
  helix_tick_queue_full_total: "驱动输入队列满载次数",
  helix_reply_fairness_forced_total: "回灌公平调度强制让步次数",
  helix_engine_loop_iteration_seconds: "引擎循环单轮耗时",
  helix_effects_per_tick: "每次驱动输入产生的效应数",
  helix_effect_bytes_per_tick: "每次驱动输入产生的效应字节数",
  helix_effect_amplification_ratio: "效应放大比",
  helix_effect_dispatch_duration_seconds: "效应分发耗时",
  helix_effect_dispatch_errors_total: "效应分发错误总数",
  helix_pool_queue_depth: "工作池队列深度",
  helix_pool_queue_capacity: "工作池队列容量",
  helix_pool_enqueue_block_seconds: "工作池入队阻塞耗时",
  helix_pool_queue_residency_seconds: "工作池队列驻留耗时",
  helix_pool_execution_seconds: "工作池执行耗时",
  helix_pool_inflight: "工作池在途任务数",
  helix_pool_dropped_total: "工作池丢弃任务总数",
  helix_http_queue_wait_seconds: "HTTP 队列等待耗时",
  helix_http_request_duration_seconds: "HTTP 请求耗时",
  helix_http_inflight: "HTTP 在途请求数",
  helix_http_request_bytes: "HTTP 请求字节数",
  helix_http_response_bytes: "HTTP 响应字节数",
  helix_http_timeout_total: "HTTP 超时总数",
  helix_http_status_total: "HTTP 状态总数",
  helix_persist_queue_wait_seconds: "持久化队列等待耗时",
  helix_storage_tx_duration_seconds: "SQLite 事务耗时",
  helix_storage_rows_total: "存储处理行数",
  helix_storage_inflight: "存储在途事务数",
  helix_storage_busy_total: "存储忙碌总数",
  helix_storage_timeout_total: "存储超时总数",
  helix_storage_rollback_total: "存储回滚总数",
  helix_storage_batch_rows: "存储批量行数",
  helix_port_roundtrip_seconds: "端口回灌往返耗时",
  helix_port_reply_queue_depth: "端口回灌队列深度",
  helix_port_reply_queue_wait_seconds: "端口回灌队列等待耗时",
  helix_port_reply_orphan_total: "端口孤儿回灌总数",
  helix_port_correlation_collision_total: "端口关联键冲突总数",
  helix_port_pending: "端口待完成请求数",
  helix_ws_ingress_to_effect_seconds: "WebSocket 入站至效应耗时",
  helix_ws_ingress_to_event_seconds: "WebSocket 入站至事件耗时",
  helix_ws_connection_state: "WebSocket 连接状态",
  helix_ws_frames_total: "WebSocket 帧总数",
  helix_ws_frame_bytes: "WebSocket 帧字节数",
  helix_ws_send_duration_seconds: "WebSocket 发送耗时",
  helix_ws_send_errors_total: "WebSocket 发送错误总数",
  helix_command_to_persist_reply_seconds: "命令至持久化回灌耗时",
  helix_command_to_immediate_projection_seconds: "命令至即时投影耗时",
  helix_command_to_http_dispatch_seconds: "命令至 HTTP 分发耗时",
  helix_command_to_http_response_seconds: "命令至 HTTP 响应耗时",
  helix_command_to_projection_emit_seconds: "命令至投影发布耗时",
  helix_event_lagged_total: "事件接收滞后总数",
  helix_event_no_receiver_total: "事件无接收者总数",
  helix_event_batch_size: "事件批大小",
  helix_event_emitted_total: "事件发布总数",
  helix_event_emit_duration_seconds: "事件发布耗时",
  helix_event_receiver_count: "事件接收者数量",
  helix_event_consumer_closed_total: "事件消费者关闭总数",
  helix_timer_active: "活跃定时器数量",
  helix_timer_lateness_seconds: "定时器逾期耗时",
  helix_timer_delivery_wait_seconds: "定时器投递等待耗时",
  helix_ffi_batch_events: "跨语言批事件数",
  helix_ffi_batch_bytes: "跨语言批字节数",
  helix_ffi_callback_duration_seconds: "跨语言回调耗时",
  helix_ffi_busy_total: "跨语言边界忙碌总数",
  helix_ffi_callback_errors_total: "跨语言回调错误总数",
  helix_ffi_events_dropped_total: "跨语言边界丢弃事件总数",
  helix_ffi_boundary_bytes: "跨语言边界字节数",
  helix_alloc_count_per_op: "每操作分配次数",
  helix_alloc_bytes_per_op: "每操作分配字节数",
  helix_process_resident_memory_bytes: "进程常驻内存",
  helix_shutdown_drain_seconds: "关闭排空耗时",
  helix_metrics_queue_depth: "指标队列深度",
  helix_metrics_queue_capacity: "指标队列容量",
  helix_metrics_dropped_total: "指标丢弃总数",
  helix_metrics_export_batch_size: "指标导出批大小",
  helix_metrics_export_duration_seconds: "指标导出耗时",
  helix_metrics_export_errors_total: "指标导出错误总数",
  helix_metrics_exporter_state: "指标导出器状态",
  helix_metrics_last_success_age_seconds: "距最近指标导出成功秒数",
  helix_traces_queue_depth: "链路队列深度",
  helix_traces_queue_capacity: "链路队列容量",
  helix_traces_dropped_total: "链路丢弃总数",
  helix_traces_export_batch_size: "链路导出批大小",
  helix_traces_export_duration_seconds: "链路导出耗时",
  helix_traces_export_errors_total: "链路导出错误总数",
  helix_traces_exporter_state: "链路导出器状态",
  helix_traces_last_success_age_seconds: "距最近链路导出成功秒数",
  helix_spans_created_total: "创建链路片段总数",
  helix_slow_spans_total: "慢链路片段总数",
  helix_lost_total: "正确性：丢失总数",
  helix_duplicate_total: "正确性：重复总数",
  helix_order_violation_total: "正确性：顺序违规总数",
  helix_wrong_channel_total: "正确性：错误频道总数",
  helix_cursor_regression_total: "正确性：游标回退总数",
  helix_projection_mismatch_total: "正确性：投影不一致总数",
  helix_persist_remaining_total: "正确性：持久化残留总数",
  helix_cleanup_residue_total: "正确性：清理残留总数",
  helix_telemetry_canary_total: "遥测金丝雀"
};

// 将内部指标名转换为 OTel Collector 暴露给 Prometheus 的指标名。
function promName(name) {
  return `otel_${name}`;
}

// 构造所有时序面板共享的中文无数据状态与布局基线。
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

// 为直方图生成中文统计图例，同时保留 PromQL 分位数合同。
function histogramPanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  const grouped = `sum by (le) (last_over_time(${metricName}_bucket{${selector}}[$rate_window]))`;
  p.targets = [
    ["中位数", "0.50"], ["P90", "0.90"], ["P95", "0.95"], ["P99", "0.99"]
  ].map(([legend, q], index) => ({ refId: String.fromCharCode(65 + index), expr: `histogram_quantile(${q}, ${grouped})`, legendFormat: legend }));
  p.targets.push({
    refId: "E",
    expr: `sum(last_over_time(${metricName}_sum{${selector}}[$rate_window])) / clamp_min(sum(last_over_time(${metricName}_count{${selector}}[$rate_window])), 0.000001)`,
    legendFormat: "平均值"
  });
  if (metric.endsWith("_seconds")) p.fieldConfig.defaults.unit = "s";
  else if (metric.includes("bytes")) p.fieldConfig.defaults.unit = "bytes";
  else p.fieldConfig.defaults.unit = "short";
  return p;
}

// 为累计计数生成速率、区间增量与累计值三种中文视图。
function counterPanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  p.targets = [
    { refId: "A", expr: `sum by (stage, operation, status) (rate(${metricName}{${selector}}[$rate_window]))`, legendFormat: "每秒速率 {{stage}} / {{operation}} / {{status}}" },
    { refId: "B", expr: `sum by (stage, operation, status) (increase(${metricName}{${selector}}[$__range]))`, legendFormat: "区间增量 {{stage}} / {{operation}} / {{status}}" },
    { refId: "C", expr: `sum by (stage, operation, status) (last_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "累计值 {{stage}} / {{operation}} / {{status}}" }
  ];
  if (metric === "helix_errors_total") {
    p.targets.push({
      refId: "D",
      expr: `sum(rate(otel_helix_operations_total{${selector},status="error"}[$rate_window])) / clamp_min(sum(rate(otel_helix_operations_total{${selector}}[$rate_window])), 0.000001)`,
      legendFormat: "错误率"
    });
  }
  p.fieldConfig.defaults.unit = metric === "helix_operations_total" ? "ops" : "short";
  return p;
}

// 为瞬时量生成当前值、平均值与最大值三种中文视图。
function gaugePanel(metric, title) {
  const p = basePanel(title);
  const metricName = promName(metric);
  p.targets = [
    { refId: "A", expr: `max by (stage, operation) (${metricName}{${selector}})`, legendFormat: "当前值 {{stage}} / {{operation}}" },
    { refId: "B", expr: `avg(avg_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "平均值" },
    { refId: "C", expr: `max(max_over_time(${metricName}{${selector}}[$rate_window]))`, legendFormat: "最大值" }
  ];
  p.fieldConfig.defaults.unit = metric.includes("bytes") ? "bytes" : metric.endsWith("_seconds") ? "s" : "short";
  return p;
}

// 构造中文领域分组行。
function row(title) {
  return { id: panelId++, title, type: "row", collapsed: false, gridPos: { h: 1, w: 24, x: 0, y: y++ }, panels: [] };
}

// 构造总览区单查询面板。
function overviewPanel(title, expr, legend, unit = "ops") {
  const p = basePanel(title);
  p.targets = [{ refId: "A", expr, legendFormat: legend }];
  p.fieldConfig.defaults.unit = unit;
  return p;
}

const panels = [];
panels.push(row("00 总览"));
const throughput = "otel_helix_run_throughput_per_second";
panels.push(overviewPanel("HTTP 每秒请求数", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="http",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("WebSocket 入站与出站每秒帧数", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="ws",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("存储每秒操作数", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="storage",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("事件每秒发布数", `max by (operation, status) (last_over_time(${throughput}{${selector},stage="event",status="ok"}[$rate_window]))`, "{{operation}} / {{status}}"));
panels.push(overviewPanel("错误总数", `sum by (stage, operation, error_kind) (last_over_time(otel_helix_errors_total{${selector}}[$rate_window]))`, "{{stage}} / {{operation}} / {{error_kind}}", "short"));
panels.push(overviewPanel("错误速率", `sum(last_over_time(${throughput}{${selector},stage="core",status="error"}[$rate_window]))`, "错误速率"));
panels.push(overviewPanel("错误率", `sum(last_over_time(${throughput}{${selector},stage="core",status="error"}[$rate_window])) / clamp_min(sum(last_over_time(${throughput}{${selector},stage="core"}[$rate_window])), 0.000001)`, "错误率", "percentunit"));

for (const rowName of [...new Set(contract.requiredMetrics.map((m) => m.row))]) {
  if (rowName !== "00 总览") panels.push(row(rowName));
  for (const metric of contract.requiredMetrics.filter((m) => m.row === rowName)) {
    const title = zhNames[metric.name] ?? metric.name;
    if (metric.kind === "histogram") panels.push(histogramPanel(metric.name, title));
    if (metric.kind === "counter") panels.push(counterPanel(metric.name, title));
    if (metric.kind === "gauge") panels.push(gaugePanel(metric.name, title));
  }
}

panels.push(row("100 正确性与金丝雀"));
for (const metric of contract.correctnessMetrics) {
  const p = counterPanel(metric, zhNames[metric]);
  p.fieldConfig.defaults.thresholds = { mode: "absolute", steps: [{ color: "green", value: null }, { color: "red", value: 0.000001 }] };
  panels.push(p);
}
const canary = basePanel("遥测金丝雀", "stat");
canary.targets = [{ refId: "A", expr: `sum(${promName(contract.canaryMetric)}{service_name=~"$service"})`, legendFormat: "金丝雀状态" }];
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
  annotations: { list: [{ builtIn: 1, datasource: { type: "grafana", uid: "-- Grafana --" }, enable: true, hide: true, iconColor: "rgba(0, 211, 255, 1)", name: "注释与告警", type: "dashboard" }] },
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
      { name: "service", label: "服务", type: "query", datasource, query: { query: "label_values(otel_helix_operations_total, service_name)", refId: "service" }, includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, refresh: 1 },
      { name: "platform", label: "平台", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service"}, platform)', refId: "platform" }, includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, refresh: 1 },
      { name: "scenario", label: "场景", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform"}, scenario)', refId: "scenario" }, includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, refresh: 1 },
      { name: "channel_shape", label: "频道形态", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform",scenario=~"$scenario"}, channel_shape)', refId: "channel_shape" }, includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, refresh: 1 },
      { name: "stage", label: "阶段", type: "custom", query: "core,effect,http,ws,storage,port_reply,event,ffi,telemetry", includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, options: [] },
      { name: "operation", label: "操作", type: "query", datasource, query: { query: 'label_values(otel_helix_operations_total{service_name=~"$service",platform=~"$platform",scenario=~"$scenario",channel_shape=~"$channel_shape",stage=~"$stage"}, operation)', refId: "operation" }, includeAll: true, allValue: ".*", current: { text: "全部", value: "$__all" }, refresh: 1 },
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
