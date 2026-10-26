#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const dir = path.dirname(fileURLToPath(import.meta.url));
const dashboard = JSON.parse(fs.readFileSync(path.join(dir, "messagev3-business-reliability.json"), "utf8"));
const rules = fs.readFileSync(path.join(dir, "recording-rules.yaml"), "utf8");
const failures = [];

// check 累积合同失败，最后一次性输出，避免只修到第一个问题。
function check(ok, message) {
  if (!ok) failures.push(message);
}

const panels = dashboard.panels.filter((panel) => panel.type !== "row");
const expressions = panels.flatMap((panel) => panel.targets ?? []).map((target) => target.expr ?? "");
const requiredRules = [
  "im:message_persist_success_ratio", "im:delivery_outbox_success_ratio",
  "im:message_remote_persist_inserted_per_second", "im:message_remote_persist_deduplicated_per_second",
  "im:recipient_enqueue_success_ratio", "im:client_ack_success_ratio",
  "helix:im_client_ack_success_ratio",
  "helix:im_projection_success_ratio", "helix:im_sync_session_success_ratio",
  "helix:im_sync_anomaly_ratio", "helix:im_recovery_success_ratio",
  "helix:im_unread_mismatch_ratio", "helix:im_message_correctness_error_ratio",
  "helix:im_command_terminal_success_ratio",
];

check(dashboard.uid === "messagev3-business-reliability", "Dashboard UID 不匹配");
check(dashboard.title === "MessageV3 业务可靠性", "Dashboard 中文标题不匹配");
check(panels.length === 14, `业务面板应为 14，实际 ${panels.length}`);
check(panels.every((panel) => panel.datasource?.uid === "prometheus"), "存在非 Prometheus 数据源面板");
for (const rule of requiredRules) {
  check(rules.includes(`record: ${rule}`), `recording rule 缺失 ${rule}`);
  if (rule !== "helix:im_command_terminal_success_ratio") {
    check(expressions.some((expr) => expr.includes(rule)), `Dashboard 未引用 ${rule}`);
  }
}
check(expressions.some((expr) => expr.includes("im_business_command_terminal_total")), "Dashboard 未引用 Go 真实 Command 终态");
check(expressions.some((expr) => expr.includes("p50_5m")), "延迟面板缺少 P50");
check(expressions.some((expr) => expr.includes("p95_5m")), "延迟面板缺少 P95");
check(expressions.some((expr) => expr.includes("p99_5m")), "延迟面板缺少 P99");
check(!rules.includes("or vector(1)"), "禁止零分母伪造 100% 成功率");
check(!dashboard.panels.some((panel) => panel.fieldConfig?.defaults?.noValue?.includes("待接入")), "Dashboard 不应继续使用待接入占位文案");
check(dashboard.templating.list.some((variable) => variable.name === "go_environment"), "Dashboard 缺少 Go 环境变量");
check(dashboard.templating.list.some((variable) => variable.name === "helix_environment"), "Dashboard 缺少 Helix 环境变量");

if (failures.length > 0) {
  for (const failure of failures) console.error(`BUSINESS DASHBOARD FAIL: ${failure}`);
  process.exit(1);
}
console.log(`business dashboard contract PASS: rules=${requiredRules.length}, panels=${panels.length}`);
