export const meta = {
  name: "dual-track-fix",
  description: "按 crate 分片并行修复(白名单零交集) → 专家对抗 review → blocking 回炉 → 主对话终验 gate.sh（分类 + 扇出 + 对抗 + 循环回炉 + gate 过滤）",
  phases: [
    { title: "Fix", detail: "每分片一 agent 在白名单内实现修复" },
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
  // Stage 1 — Fix（默认子 agent 搬砖，便宜可大 fan-out）
  (s) =>
    agent(
      `你是 helix 修复 agent，分片 crate=${s.crate}。仓库根 ${REPO}。\n任务：${s.task}\n` +
        `**文件白名单（只能改这些，一个都不许越界）**：\n${(s.whitelist || []).map((p) => "- " + p).join("\n")}\n` +
        `约束：遵守根 CLAUDE.md + .claude/rules/runtime-agnostic-core.md + 命中的 docs/harness/HX-C*.md；边界输入零 unwrap/expect；热路径 O(1) 禁 read-modify-write；` +
        `helix-core 改动禁 import tokio/sqlx/reqwest/web-sys/tauri。**中途禁止反复 cargo check/test 循环**（终验留给主对话）。改完返回改了哪些文件。`,
      { label: `fix:${s.crate}`, phase: "Fix", schema: FIX_SCHEMA }
    ),
  // Stage 2 — Review（专家对抗，按 lens 选人）
  (fix, s) => {
    if (!fix) return { crate: s.crate, status: "fix-failed" };
    const reviewerType = REVIEWER[s.lens || "rust"] || "rust-dev";
    return agent(
      `对抗 review helix 分片 crate=${s.crate} 的改动。仓库根 ${REPO}。\n改动文件：${(fix.filesTouched || []).join(", ")}\n` +
        `用 \`git diff -- ${(fix.filesTouched || []).join(" ")}\` 看 diff。逐条挑 must-fix：正确性 bug、越白名单、边界 unwrap/expect、热路径分配/RMW、` +
        `core 误引运行时依赖、Send/!Send 错误、Effect 未批处理、确定性破坏(core 内调 now/rand)。无 must-fix 则 ok=true。`,
      { label: `review:${s.crate}`, phase: "Review", agentType: reviewerType, schema: REVIEW_SCHEMA }
    ).then((rev) => ({ crate: s.crate, whitelist: s.whitelist, fix, review: rev }));
  },
  // Stage 3 — Repair（blocking 回炉，only 当有 must-fix）
  (r) => {
    if (!r || r.status === "fix-failed") return r;
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
const blocked = reports.filter((r) => r && (r.status === "needs-human" || r.status === "fix-failed"));
log(
  `分片完成: ${done.length} clean/repaired, ${blocked.length} 需人工。主对话下一步: git diff 审总改动 → scripts/gate.sh 终验 → 无交集按 crate 分 commit。`
);
return {
  mergeReady: done.map((r) => ({ crate: r.crate, files: r.fix.filesTouched })),
  blocked: blocked.map((r) => ({ crate: r.crate, status: r.status })),
  nextAction:
    "主对话(不在 workflow 内做): 1) git diff 审总改动 2) scripts/gate.sh 终验 3) 分片白名单零交集→按 crate 分 commit；blocked 分片留人工",
};
