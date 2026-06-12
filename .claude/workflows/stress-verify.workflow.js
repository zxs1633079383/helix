export const meta = {
  name: "stress-verify",
  description: "性能/承压主张实测随队：多场景跑数 → core-dev+rust-dev 对抗审方法学 → 无数字/方法学不成立不收（生成-过滤 + 对抗验证）",
  phases: [
    { title: "Measure", detail: "每场景一 agent 跑 bench/replay/压测出数字" },
    { title: "Audit", detail: "core-dev + rust-dev 对抗审基准方法学" },
  ],
};

// args: { claim: string, scenarios: [{name, cmd, expect?}] }
const REPO = "/Users/mac28/workspace/rustWorkspace/helix";
// ⚠️ 本运行时把 Workflow 的 args 作为 JSON 字符串注入（实测 typeof args === "string"），非已解析对象。
const ARGS = (() => {
  let a = typeof args !== "undefined" ? args : {};
  if (typeof a === "string") {
    try { a = JSON.parse(a); } catch (_e) { a = {}; }
  }
  return a && typeof a === "object" ? a : {};
})();
const claim = ARGS.claim || null;
const scenarios = ARGS.scenarios || [];
if (!claim) return { error: "args.claim required（要验证的性能/承压主张）" };
if (scenarios.length === 0)
  return { error: "args.scenarios required: [{name, cmd, expect?}]" };

const MEASURE_SCHEMA = {
  type: "object",
  required: ["scenario", "ran", "metrics"],
  properties: {
    scenario: { type: "string" },
    ran: { type: "boolean", description: "命令是否真跑成功" },
    metrics: {
      type: "array",
      items: {
        type: "object",
        required: ["name", "value", "unit"],
        properties: {
          name: { type: "string" },
          value: { type: "number" },
          unit: { type: "string" },
        },
      },
    },
    raw: { type: "string", description: "关键输出原文摘录（criterion/nextest 行）；失败则放错误" },
  },
};
const AUDIT_SCHEMA = {
  type: "object",
  required: ["accept", "reason"],
  properties: {
    accept: { type: "boolean", description: "方法学成立且数字支持主张" },
    reason: { type: "string" },
    flaws: { type: "array", items: { type: "string" } },
  },
};

// Measure — 各场景并行实跑（autonomous 模式，agent 自跑命令）
const measured = await parallel(
  scenarios.map((sc) => () =>
    agent(
      `你是 helix 压测 agent。仓库根 ${REPO}（autonomous 模式，你用 Bash 自跑命令）。\n场景：${sc.name}\n执行：\`${sc.cmd}\`\n` +
        (sc.expect ? `期望：${sc.expect}\n` : "") +
        `跑该命令（criterion bench / cargo nextest / 自定义压测）。摘录关键数字行填 metrics。命令失败则 ran=false 并把错误放 raw。`,
      { label: `measure:${sc.name}`, phase: "Measure", schema: MEASURE_SCHEMA }
    )
  )
);

const good = measured.filter(Boolean).filter((m) => m.ran);
if (good.length === 0) {
  log("⛔ 无任何成功跑出的数字 — 性能主张不收（无数字不收）。");
  return { claim, verdict: "REJECT", reason: "无成功测得的数字", measured: measured.filter(Boolean) };
}

// Audit — 双专家对抗审方法学；坚固 core 用 AND 闸门（两人都过才收）
const audit = await parallel(
  ["rust-runtime-agnostic-core-dev", "rust-dev"].map((at) => () =>
    agent(
      `对抗审计 helix 性能主张的方法学。仓库根 ${REPO}。\n主张：${claim}\n实测数据：\n${good
        .map((m) => `- ${m.scenario}: ${m.metrics.map((x) => x.name + "=" + x.value + x.unit).join(", ")} | ${m.raw}`)
        .join("\n")}\n` +
        `审：基准是否量到热路径？是否真 O(1) 而非被 cache 掩盖？有无隐藏分配/RMW？样本量与 warmup 够不够？数字是否真支持主张？\n` +
        `你的角色：${at === "rust-dev" ? "热路径/分配/并发正确性" : "三栖一致性——同一结论在 wasm/FFI 是否同样成立"}。方法学不成立或数字不支持 → accept=false。`,
      { label: `audit:${at}`, phase: "Audit", agentType: at, schema: AUDIT_SCHEMA }
    )
  )
);

const a = audit.filter(Boolean);
const accepted = a.length > 0 && a.every((x) => x.accept);
const verdict = accepted ? "ACCEPT" : "REJECT";
log(`性能主张 ${verdict}（${a.filter((x) => x.accept).length}/${a.length} 审计通过）`);
return { claim, verdict, measured: good, audit: a, flaws: a.flatMap((x) => x.flaws || []) };
