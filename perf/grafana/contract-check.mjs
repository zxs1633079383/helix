#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const contract = JSON.parse(fs.readFileSync(path.join(dir, "dashboard.contract.json"), "utf8"));
const dashboard = JSON.parse(fs.readFileSync(path.join(dir, "helix-runtime-performance-pre.json"), "utf8"));
const failures = [];
const panels = dashboard.panels.filter((panel) => panel.type !== "row");
const targets = panels.flatMap((panel) => (panel.targets ?? []).map((target) => ({ panel, ...target })));
const expressions = targets.map((target) => target.expr ?? "");
const variables = dashboard.templating?.list?.map((item) => item.name) ?? [];

function check(ok, message) {
  if (!ok) failures.push(message);
}

function metricTargets(name) {
  const exported = `otel_${name}`;
  return targets.filter((target) => (target.expr ?? "").includes(exported));
}

check(contract.requiredMetrics.length === 38, `requiredMetrics 必须恰好 38，实际 ${contract.requiredMetrics.length}`);
check(dashboard.uid === contract.uid, "Dashboard UID 不匹配");
check(dashboard.title === contract.title, "Dashboard title 不匹配");
check(dashboard.templating.list[0]?.current?.value === contract.datasourceUid, "默认 datasource UID 不匹配");
for (const variable of contract.variables) check(variables.includes(variable), `缺少变量 $${variable}`);

for (const metric of contract.requiredMetrics) {
  const found = metricTargets(metric.name);
  check(found.length > 0, `指标 ${metric.name} 未被任何 panel 引用`);
  if (metric.kind === "histogram") {
    const panel = found[0]?.panel;
    const panelTargets = panel?.targets ?? [];
    const legends = new Set(panelTargets.map((target) => target.legendFormat));
    for (const statistic of contract.histogramStatistics) check(legends.has(statistic), `${metric.name} 缺少 ${statistic}`);
    const panelExpr = panelTargets.map((target) => target.expr).join("\n");
    for (const token of ["0.50", "0.90", "0.95", "0.99", `${metric.name}_sum`, `${metric.name}_count`]) {
      check(panelExpr.includes(token), `${metric.name} 缺少 histogram token ${token}`);
    }
  }
  if (metric.kind === "counter") {
    const expr = found.map((target) => target.expr).join("\n");
    check(expr.includes("rate("), `${metric.name} 缺少 rate()`);
    check(expr.includes("increase("), `${metric.name} 缺少 increase()`);
  }
  if (metric.kind === "gauge") {
    const legends = new Set(found.map((target) => target.legendFormat));
    for (const statistic of contract.gaugeStatistics) {
      check([...legends].some((legend) => legend?.startsWith(statistic)), `${metric.name} 缺少 ${statistic}`);
    }
  }
}

for (const metric of contract.correctnessMetrics) check(metricTargets(metric).length > 0, `正确性指标 ${metric} 未覆盖`);
check(metricTargets(contract.canaryMetric).length > 0, "canary 指标未覆盖");
check(expressions.some((expr) => expr.includes('stage="http"') && expr.includes("rate(")), "缺少 HTTP RPS");
check(expressions.some((expr) => expr.includes('stage="ws"') && expr.includes("direction")), "缺少 WS inbound/outbound RPS");
check(expressions.some((expr) => expr.includes('stage="storage"') && expr.includes("rate(")), "缺少 Storage QPS");
check(expressions.some((expr) => expr.includes('stage="event"') && expr.includes("rate(")), "缺少 Event RPS");
check(expressions.some((expr) => expr.includes("status=\"error\"") && expr.includes("clamp_min")), "缺少错误率");
check(expressions.some((expr) => expr.includes("otel_helix_errors_total") && expr.includes("increase(")), "缺少错误总数");
check(expressions.some((expr) => expr.includes("otel_helix_errors_total") && expr.includes("rate(")), "缺少错误速率");
for (const panel of panels) check(panel.datasource?.uid === contract.datasourceUid, `面板 ${panel.title} datasource UID 错误`);

const queryDigest = crypto.createHash("sha256").update([...expressions].sort().join("\n")).digest("hex");
if (failures.length > 0) {
  for (const failure of failures) console.error(`CONTRACT FAIL: ${failure}`);
  process.exit(1);
}
console.log(`dashboard contract PASS: 38 metrics + 8 correctness + canary, panels=${panels.length}, queryDigest=${queryDigest}`);
