export const meta = {
  name: "architecture-drift-review",
  description: "对 <last_reviewed_commit>..HEAD 的 diff 跑架构不偏移 review：HX-C001..C008 不变量 + GOAL §1/§2 + 全流程合理性，对抗证伪后给 drift 判定 + 收敛 todo + ledger 更新指令",
  phases: [
    { title: "Scope", detail: "从 ledger 取 last_reviewed_commit，算 diff 区间" },
    { title: "Review", detail: "4 专家 + GOAL/全流程 lens 并行查 drift" },
    { title: "Verify", detail: "每条疑似 drift 对抗证伪" },
    { title: "Synthesize", detail: "汇总 drift 判定 + 收敛动作 + ledger 更新指令" },
  ],
};

const REPO = "/Users/mac28/workspace/rustWorkspace/helix";
// args 归一化（运行时把 args 作 JSON 字符串注入，与其它 workflow 同款 shim）
const ARGS = (() => {
  let a = typeof args !== "undefined" ? args : {};
  if (typeof a === "string") { try { a = JSON.parse(a); } catch (_e) { a = {}; } }
  return a && typeof a === "object" ? a : {};
})();
const baseOverride = ARGS.base || null; // 可选：显式指定 base commit，否则读 ledger

phase("Scope");
const SCOPE_SCHEMA = {
  type: "object", additionalProperties: false,
  required: ["base", "head", "changed_files", "diff_summary"],
  properties: {
    base: { type: "string", description: "last_reviewed_commit（或 args.base）" },
    head: { type: "string", description: "当前 HEAD short sha" },
    changed_files: { type: "string", description: "git diff --stat base..HEAD 全文" },
    diff_summary: { type: "string", description: "改动按子系统归类的概述（parser/cursor/storage/host/driver/...），含净增减" },
  },
};
const scope = await agent(`cwd=${REPO}。算本次架构 review 的 diff 区间。
${baseOverride ? `base 已指定 = ${baseOverride}` : "base = docs/review/workflow-review-ledger.md 里 `last_reviewed_commit:` 的值"}。
跑：git rev-parse --short HEAD；git diff --stat <base>..HEAD；git diff <base>..HEAD（看实质改动）。
若 base==HEAD（gap=0，无新 commit）→ diff_summary 写 "NO_NEW_COMMITS"。
返回 schema。`,
  { label: "scope:diff-range", phase: "Scope", agentType: "Explore", schema: SCOPE_SCHEMA });

if (!scope || /NO_NEW_COMMITS/.test(scope.diff_summary || "")) {
  log("无新 commit（gap=0），跳过 review");
  return { verdict: "🟢 no-op", reason: "base==HEAD，无可 review 的改动", scope };
}
const RANGE = `${scope.base}..${scope.head}`;
log(`review 区间 ${RANGE}：\n${scope.changed_files}`);

phase("Review");
const FINDINGS_SCHEMA = {
  type: "object", additionalProperties: false,
  required: ["findings"],
  properties: {
    findings: {
      type: "array",
      items: {
        type: "object", additionalProperties: false,
        required: ["title", "where", "severity", "invariant", "why"],
        properties: {
          title: { type: "string" },
          where: { type: "string", description: "文件:行" },
          severity: { type: "string", enum: ["high", "medium", "low"] },
          invariant: { type: "string", description: "违反了哪条：HX-C001..C008 / GOAL §1 / GOAL §2 / 全流程接缝" },
          why: { type: "string", description: "为什么是 drift + 收敛方向" },
        },
      },
    },
  },
};
// ⚠️ READ-ONLY 铁律：drift-review 是审查，不是施工。review workflow 越权改文件 = 引入未审 drift。
const READONLY = `\n\n# ⛔ READ-ONLY（铁律，违反即污染）\n你是审查者，**只读不写**：禁止 Edit/Write/新建/删除/重构任何文件，禁止 git add/commit/checkout，禁止跑会改文件的命令。\n只能 git diff / grep / cat / Read 看代码，产出结论文本。发现问题→写进 findings（文件:行 + 收敛方向），由主对话决定怎么改。绝不自己动手"顺便修"。`;
const COMMON = `cwd=${REPO}。只审 diff 区间 ${RANGE}（git diff ${RANGE}）。找**架构偏移**，不是普通 bug。
每条给 文件:行 + severity(high/medium/low) + 违反的不变量 + 收敛方向。无 drift → findings 空数组。${READONLY}`;

const LENSES = [
  { key: "core-tri", agentType: "rust-runtime-agnostic-core-dev",
    lens: `HX-C001(helix-core 零运行时/零业务 schema：grep core 不得现 tokio/sqlx/reqwest/tauri/CREATE TABLE/业务表名) · HX-C002(async port Send 边界 + 状态禁 &'a) · HX-C004(features additive) · HX-C006(FFI 边界) · HX-C007(Effect 跨边界批处理) · GOAL §2 硬约束(三层依赖单向 adapter→core、sans-IO、确定性 time/id 注入)` },
  { key: "rust-hotpath", agentType: "rust-dev",
    lens: `HX-C005(热路径 O(1)、&'static str 不 String、禁 read-modify-write) · Rust 正确性(借用/Send/取消安全) · 边界输入零 unwrap/expect · thiserror 边界` },
  { key: "prod-flow", agentType: "rust-tauri-skeleton-dev",
    lens: `现网对齐(cses-client 唯一权威) · HX-C008(sync/notify pull 契约：cursor 锚 nextSeq、续拉 len≥500、真 wire entries) · **全流程接缝合理性**：收发(WS/HTTP)→解析(parser)→cursor(gate/续拉)→落库(schema/列对齐) 是否自洽、有无死代码/绕过/临时 mock 残留/表名列名漂移` },
  { key: "goal", agentType: "rust-dev",
    lens: `读 docs/GOAL.md §1 设计理念(不变量) + §2 硬约束，逐条对照 diff 是否偏离(runtime-agnostic、YAGNI 但边界对、IM 只是第一个业务模块、core 不认业务)` },
];
const reviews = await parallel(LENSES.map((L) => () =>
  agent(`${COMMON}\n# 你的 lens（${L.key}）\n${L.lens}`,
    { label: `review:${L.key}`, phase: "Review", agentType: L.agentType, schema: FINDINGS_SCHEMA, effort: "high" })
));
const allFindings = reviews.filter(Boolean).flatMap((r) => r.findings || []);
log(`收集 ${allFindings.length} 条疑似 drift`);

phase("Verify");
const VERDICT_SCHEMA = {
  type: "object", additionalProperties: false,
  required: ["is_real", "severity", "verdict"],
  properties: {
    is_real: { type: "boolean", description: "是否确为架构偏移（默认怀疑，证不实则 false）" },
    severity: { type: "string", enum: ["high", "medium", "low", "not-a-drift"] },
    verdict: { type: "string", description: "证伪结论 + 收敛动作" },
  },
};
const verified = await parallel(allFindings.map((f) => () =>
  agent(`cwd=${REPO}。对抗证伪这条疑似架构偏移（默认它**不是** drift，除非你能用 ${RANGE} 的代码证据证实）：
标题：${f.title}
位置：${f.where}
违反：${f.invariant}
理由：${f.why}
去看真实代码 + 对照 docs/harness/${f.invariant} 或 docs/GOAL.md。给 is_real + severity + 收敛动作。${READONLY}`,
    { label: `verify:${(f.where || f.title).slice(0, 32)}`, phase: "Verify", schema: VERDICT_SCHEMA, effort: "high" })
    .then((v) => ({ ...f, verdict: v }))
));
const confirmed = verified.filter(Boolean).filter((f) => f.verdict && f.verdict.is_real);

phase("Synthesize");
const worst = confirmed.reduce((acc, f) => {
  const rank = { high: 3, medium: 2, low: 1 }[f.verdict.severity] || 0;
  return Math.max(acc, rank);
}, 0);
const verdictTag = worst >= 3 ? "🔴 重 drift" : worst === 2 ? "🟡 轻 drift" : confirmed.length ? "🟡 轻 drift" : "🟢 无 drift";

const SYNTH_SCHEMA = {
  type: "object", additionalProperties: false,
  required: ["verdict", "converge_todo", "ledger_update", "summary"],
  properties: {
    verdict: { type: "string", description: "🟢无/🟡轻/🔴重 drift + 一句话" },
    converge_todo: { type: "string", description: "收敛动作清单（🔴 必须先修再更新 ledger；🟡 记录+择期修；🟢 无）" },
    ledger_update: { type: "string", description: `给主对话的 ledger 更新指令：把 last_reviewed_commit 改为 ${scope.head} + 追历史行（含区间/runId/判定）。🔴 时注明须修复回正后再指向已收敛 commit` },
    summary: { type: "string", description: "本次 review 总结" },
  },
};
const synth = await agent(`cwd=${REPO}。汇总架构 drift review（区间 ${RANGE}）。
预判级别：${verdictTag}。确认的 drift（${confirmed.length} 条）：
${confirmed.map((f) => `- [${f.verdict.severity}] ${f.title} @${f.where}（${f.invariant}）→ ${f.verdict.verdict}`).join("\n") || "（无确认 drift）"}

产出：verdict + converge_todo + ledger_update + summary。
铁律：workflow 不改 ledger/不 commit（主对话做）；ledger_update 是给主对话的指令文本。${READONLY}`,
  { label: "synthesize", phase: "Synthesize", schema: SYNTH_SCHEMA, effort: "high" });

return {
  range: RANGE,
  verdict: synth?.verdict || verdictTag,
  confirmed_drift: confirmed.map((f) => ({ title: f.title, where: f.where, severity: f.verdict.severity, invariant: f.invariant })),
  converge_todo: synth?.converge_todo,
  ledger_update: synth?.ledger_update,
  summary: synth?.summary,
  raw_findings_count: allFindings.length,
};
