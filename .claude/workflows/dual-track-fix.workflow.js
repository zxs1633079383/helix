export const meta = {
  name: "dual-track-fix",
  description: "按 crate 分片：plan gate(动手前出 spec 块) → 并行修复(白名单零交集·按 spec 实现) → 专家对抗 review → blocking 回炉 → 主对话终验 gate.sh",
  phases: [
    { title: "Plan", detail: "每分片先出 spec 块（动手前·plan gate·对标 Trend 1）" },
    { title: "Fix", detail: "每分片一 agent 按已出 spec 在白名单内实现修复" },
    { title: "Review", detail: "专家对抗 review 分片 diff" },
    { title: "Repair", detail: "blocking 回炉修复 must-fix" },
  ],
};

// args: { shards: [{crate, whitelist:[paths], task, lens?}], baseline?: sha }
// lens ∈ {rust, core, align}（选 review 专家）；缺省 rust
const REPO = "/Users/mac28/workspace/rustWorkspace/helix";
// ⚠️ 本运行时把 Workflow 的 args 作为 JSON 字符串注入（实测 typeof args === "string"），非已解析对象。
const ARGS = (() => {
  let a = typeof args !== "undefined" ? args : {};
  if (typeof a === "string") {
    try { a = JSON.parse(a); } catch (_e) { a = {}; }
  }
  return a && typeof a === "object" ? a : {};
})();
const SHARDS = ARGS.shards || [];
if (SHARDS.length === 0)
  return { error: "args.shards required: [{crate, whitelist:[...], task, lens?}]" };

// 前置铁律：白名单零交集校验（有交集即不可并行，项目 §6 规则）
const seen = new Map();
for (const s of SHARDS) {
  for (const p of s.whitelist || []) {
    if (seen.has(p))
      return { error: `白名单交集冲突: ${p} 同属 ${seen.get(p)} 与 ${s.crate} — 拒绝并行编排` };
    seen.set(p, s.crate);
  }
}

const REVIEWER = {
  rust: "rust-dev",
  core: "rust-runtime-agnostic-core-dev",
  align: "rust-tauri-skeleton-dev",
};

// 🚦 Plan Gate spec 块（动手前必出·对标 Trend 1「Agent understands→显式 spec→implements」+ spec-driven-development skill）。
// 见 README §Plan Gate。任何 file-modifying 阶段前必出此块；schema 强制，缺字段=该 agent 直接 FAIL。
const PLAN_GATE_SCHEMA = {
  type: "object",
  required: ["objective", "filesToChange", "approach", "acceptance"],
  properties: {
    objective: { type: "string", description: "要改什么 + 为什么（理解到的 intent，可证伪）" },
    filesToChange: {
      type: "array",
      items: {
        type: "object",
        required: ["file", "what"],
        properties: { file: { type: "string" }, what: { type: "string" } },
      },
      description: "改哪些文件·每文件改什么（须全在白名单内）",
    },
    approach: { type: "string", description: "技术方案：怎么改（不是做什么，是怎么实现）" },
    assumptions: { type: "array", items: { type: "string" }, description: "浮出的假设（默默填=最危险）" },
    acceptance: {
      type: "array",
      items: {
        type: "object",
        required: ["criterion", "verify"],
        properties: { criterion: { type: "string" }, verify: { type: "string" } },
      },
      description: "可测成功条件 + 验证命令（如 cargo test -p x / grep …）",
    },
    boundaries: { type: "string", description: "绝不碰什么 + 风险点" },
    openQuestions: { type: "array", items: { type: "string" }, description: "需人类拍板才能继续的（非空=应回主对话）" },
  },
};

const FIX_SCHEMA = {
  type: "object",
  required: ["filesTouched", "summary"],
  properties: {
    filesTouched: { type: "array", items: { type: "string" } },
    summary: { type: "string" },
    outOfScope: { type: "boolean", description: "true 若被迫改了白名单外文件（异常，应避免）" },
  },
};
const REVIEW_SCHEMA = {
  type: "object",
  required: ["ok", "issues"],
  properties: {
    ok: { type: "boolean" },
    issues: {
      type: "array",
      items: {
        type: "object",
        required: ["detail", "severity"],
        properties: {
          detail: { type: "string" },
          fix: { type: "string" },
          severity: { enum: ["must-fix", "should-fix", "nit"] },
        },
      },
    },
  },
};
const REPAIR_SCHEMA = {
  type: "object",
  required: ["applied", "remaining"],
  properties: {
    applied: { type: "integer" },
    remaining: { type: "integer", description: "未能解决的 must-fix 数" },
    note: { type: "string" },
  },
};

const reports = await pipeline(
  SHARDS,
  // Stage 1 — Plan（🚦 plan gate：动手前先出 spec 块，**只读+规划，禁改任何文件**）
  (s) =>
    agent(
      `你是 helix 规划 agent，分片 crate=${s.crate}。仓库根 ${REPO}。\n任务：${s.task}\n` +
        `**文件白名单（修复阶段只能改这些）**：\n${(s.whitelist || []).map((p) => "- " + p).join("\n")}\n` +
        `🚦 **Plan Gate（对标 Trend 1 + spec-driven-development skill）**：你现在**只读代码 + 出 spec 块，绝不改任何文件**。\n` +
        `读相关代码，按 schema 产出 spec：objective(改什么+为什么) / filesToChange(每文件改什么·须在白名单内) / approach(怎么实现) / ` +
        `assumptions(浮出假设·别默默填) / acceptance(可测条件+验证命令) / boundaries(绝不碰+风险) / openQuestions(需人类拍板的)。\n` +
        `约束认知：根 CLAUDE.md + runtime-agnostic-core.md + 命中的 HX-C*；core 禁运行时依赖；热路径 O(1) 零 RMW；边界零 unwrap。`,
      { label: `plan:${s.crate}`, phase: "Plan", schema: PLAN_GATE_SCHEMA }
    ),
  // Stage 2 — Fix（按已出 spec 实现；plan 缺失/有未决 openQuestions 则不动手）
  (plan, s) => {
    if (!plan) return { crate: s.crate, status: "plan-failed" };
    if ((plan.openQuestions || []).length > 0)
      return { crate: s.crate, status: "blocked-on-questions", plan };
    return agent(
      `你是 helix 修复 agent，分片 crate=${s.crate}。仓库根 ${REPO}。\n` +
        `**按你刚出的 spec 实现**（plan gate 已过）：\n${JSON.stringify(plan, null, 2)}\n` +
        `**文件白名单（只能改这些，一个都不许越界）**：\n${(s.whitelist || []).map((p) => "- " + p).join("\n")}\n` +
        `严格按 spec 的 filesToChange + approach 改；偏离 spec 需在返回里说明。约束：边界零 unwrap/expect；热路径 O(1) 禁 RMW；` +
        `helix-core 禁 import tokio/sqlx/reqwest/web-sys/tauri。**中途禁止反复 cargo check/test 循环**（终验留主对话）。改完返回改了哪些文件。`,
      { label: `fix:${s.crate}`, phase: "Fix", schema: FIX_SCHEMA }
    ).then((fix) => (fix ? { ...fix, plan } : fix));
  },
  // Stage 3 — Review（专家对抗，按 lens 选人）
  (fix, s) => {
    if (!fix) return { crate: s.crate, status: "fix-failed" };
    if (fix.status === "plan-failed" || fix.status === "blocked-on-questions") return fix;
    const reviewerType = REVIEWER[s.lens || "rust"] || "rust-dev";
    return agent(
      `对抗 review helix 分片 crate=${s.crate} 的改动。仓库根 ${REPO}。\n改动文件：${(fix.filesTouched || []).join(", ")}\n` +
        `用 \`git diff -- ${(fix.filesTouched || []).join(" ")}\` 看 diff。逐条挑 must-fix：正确性 bug、越白名单、边界 unwrap/expect、热路径分配/RMW、` +
        `core 误引运行时依赖、Send/!Send 错误、Effect 未批处理、确定性破坏(core 内调 now/rand)。无 must-fix 则 ok=true。`,
      { label: `review:${s.crate}`, phase: "Review", agentType: reviewerType, schema: REVIEW_SCHEMA }
    ).then((rev) => ({ crate: s.crate, whitelist: s.whitelist, fix, review: rev }));
  },
  // Stage 4 — Repair（blocking 回炉，only 当有 must-fix）
  (r) => {
    if (!r || r.status === "fix-failed" || r.status === "plan-failed" || r.status === "blocked-on-questions")
      return r;
    const mustFix = (r.review && r.review.issues ? r.review.issues : []).filter(
      (i) => i.severity === "must-fix"
    );
    if (r.review && r.review.ok && mustFix.length === 0) return { ...r, status: "clean" };
    return agent(
      `回炉修复 helix 分片 crate=${r.crate}。仓库根 ${REPO}。只改白名单内文件：\n${(r.whitelist || []).join("\n")}\n` +
        `must-fix 列表：\n${mustFix
          .map((i, n) => `${n + 1}. [${i.severity}] ${i.detail}${i.fix ? " → 修法:" + i.fix : ""}`)
          .join("\n")}\n逐条修，返回 applied/remaining。`,
      { label: `repair:${r.crate}`, phase: "Repair", schema: REPAIR_SCHEMA }
    ).then((rep) => ({
      ...r,
      repair: rep,
      status: rep && rep.remaining === 0 ? "repaired" : "needs-human",
    }));
  }
);

const done = reports.filter((r) => r && (r.status === "clean" || r.status === "repaired"));
const blocked = reports.filter(
  (r) =>
    r &&
    (r.status === "needs-human" ||
      r.status === "fix-failed" ||
      r.status === "plan-failed" ||
      r.status === "blocked-on-questions")
);
log(
  `分片完成: ${done.length} clean/repaired, ${blocked.length} 需人工（含 plan gate 拦下的 blocked-on-questions）。主对话下一步: git diff 审总改动 → scripts/gate.sh 终验 → 无交集按 crate 分 commit。`
);
return {
  mergeReady: done.map((r) => ({ crate: r.crate, files: r.fix.filesTouched })),
  blocked: blocked.map((r) => ({ crate: r.crate, status: r.status })),
  nextAction:
    "主对话(不在 workflow 内做): 1) git diff 审总改动 2) scripts/gate.sh 终验 3) 分片白名单零交集→按 crate 分 commit；blocked 分片留人工",
};
