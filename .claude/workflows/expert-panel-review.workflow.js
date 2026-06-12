export const meta = {
  name: "expert-panel-review",
  description: "四专家并行审查 helix 设计/diff/方案 → 每条发现对抗证伪 → 汇总确认项（扇出-综合 + 对抗验证）",
  phases: [
    { title: "Review", detail: "4 专家各带 lens 并行出 findings" },
    { title: "Verify", detail: "每专家 top-2 发现各 2 票对抗证伪" },
  ],
};

// args: { target: string(文件路径或方案描述), context?: string }
const REPO = "/Users/mac28/workspace/rustWorkspace/helix";
// ⚠️ 本运行时把 Workflow 的 args 作为 JSON 字符串注入（实测 typeof args === "string"），非已解析对象。
// 统一归一化：字符串就 JSON.parse，缺省 {}。所有 workflow 脚本共用此 shim。
const ARGS = (() => {
  let a = typeof args !== "undefined" ? args : {};
  if (typeof a === "string") {
    try { a = JSON.parse(a); } catch (_e) { a = {}; }
  }
  return a && typeof a === "object" ? a : {};
})();
const target = ARGS.target || null;
if (!target) return { error: "args.target required（评审对象：文件路径或方案描述）" };
const context = ARGS.context || "";

// 4 专家固定编制；dioxus-dev 显式锁死「只 core 内核经验，禁渲染层」（项目约定）
const EXPERTS = [
  {
    key: "rust",
    agentType: "rust-dev",
    lens: "Rust 语言与工程正确性：所有权/借用/Send 边界、async 取消安全、错误处理(thiserror 边界 + 边界输入零 unwrap)、热路径分配与 O(1)、unsafe 健全性、trait 设计权衡。",
  },
  {
    key: "tri-target",
    agentType: "rust-runtime-agnostic-core-dev",
    lens: "三栖边界守门：helix-core 零运行时依赖(HX-C001)、async port Send/!Send cfg 切(HX-C002)、wasm cfg 分支必须 CI type-check(HX-C003)、features additive(HX-C004)、FFI 边界(HX-C006)、Effect 跨边界批处理(HX-C007)、OPFS/wasm 存储差异。",
  },
  {
    key: "prod-align",
    agentType: "rust-tauri-skeleton-dev",
    lens: "现网对齐（唯一权威 = cses-client IM handler）：IPC 契约、增量同步 cursor 单调性、DbManager 读写分离 + 热路径 O(1) 落库、灰度换引擎接缝。指出与现网真实行为的偏差。",
  },
  {
    key: "kernel",
    agentType: "dioxus-dev",
    lens: "【仅取 core 内核经验，禁止涉及 RSX/组件/渲染器/UI 层任何话题，提到就算跑题】step 循环重入安全、Effect 批次边界、调度公平性与脏标记、corr_map 不变量、one-shot timer 语义、GenerationalBox/Copy 状态所有权对照。",
  },
];

const FINDINGS_SCHEMA = {
  type: "object",
  required: ["findings"],
  properties: {
    findings: {
      type: "array",
      items: {
        type: "object",
        required: ["title", "severity", "detail"],
        properties: {
          title: { type: "string" },
          severity: { enum: ["critical", "high", "medium", "nit"] },
          detail: { type: "string", description: "问题是什么 + 在哪（文件:行 或 节）" },
          suggestion: { type: "string" },
          location: { type: "string" },
        },
      },
    },
  },
};

const VERDICT_SCHEMA = {
  type: "object",
  required: ["real", "reason"],
  properties: {
    real: { type: "boolean", description: "true=确实是真问题；false=可证伪/误报/已被现有约束覆盖" },
    reason: { type: "string" },
  },
};

const SEV_RANK = { critical: 0, high: 1, medium: 2, nit: 3 };

// pipeline：每个专家审完立刻进证伪，不等其他专家（无 barrier）
const results = await pipeline(
  EXPERTS,
  (e) =>
    agent(
      `你是 helix 专家评审（视角=${e.key}）。审查对象：${target}\n` +
        (context ? `补充上下文：${context}\n` : "") +
        `\n仓库根：${REPO}。先 Read 评审对象 + 相关规则（根 CLAUDE.md / docs/GOAL.md / .claude/rules/runtime-agnostic-core.md / 命中的 docs/harness/HX-C*.md）。` +
        `\n你的评审视角（只在这个 lens 内挑问题，不越界）：${e.lens}` +
        `\n只报你这个视角下的真问题，按 severity 标级。没问题就返回空 findings。`,
      { label: `review:${e.key}`, phase: "Review", agentType: e.agentType, schema: FINDINGS_SCHEMA }
    ),
  (review, e) => {
    const top = (review && review.findings ? [...review.findings] : [])
      .sort((a, b) => SEV_RANK[a.severity] - SEV_RANK[b.severity])
      .slice(0, 2);
    return parallel(
      top.flatMap((f) =>
        [0, 1].map((i) => () =>
          agent(
            `对抗证伪。helix 评审发现一条问题，请尽力反驳（默认倾向可证伪）：\n` +
              `标题：${f.title}\n详情：${f.detail}\n位置：${f.location || "—"}\n` +
              `Read 仓库根 ${REPO} 下相关文件核实。若不成立/误报/已被现有约束覆盖 → real=false 给理由；只有确凿成立才 real=true。`,
            { label: `verify:${e.key}:${i}`, phase: "Verify", schema: VERDICT_SCHEMA }
          )
        )
      )
    ).then((votes) => {
      const v = (votes || []).filter(Boolean);
      // 稳固 core 偏保守：2 票中只要有 1 票无法证伪即保留该发现（宁可留给人工，不轻易丢）
      const confirmed = top.filter((_, idx) => {
        const pair = [v[idx * 2], v[idx * 2 + 1]].filter(Boolean);
        return pair.length === 0 || pair.some((x) => x.real);
      });
      return { expert: e.key, confirmed, dropped: top.length - confirmed.length };
    });
  }
);

const flat = results.filter(Boolean);
const allConfirmed = flat.flatMap((r) => r.confirmed.map((f) => ({ ...f, expert: r.expert })));
allConfirmed.sort((a, b) => SEV_RANK[a.severity] - SEV_RANK[b.severity]);
log(`确认 ${allConfirmed.length} 条（critical/high 优先），证伪丢弃 ${flat.reduce((a, r) => a + r.dropped, 0)} 条`);
return { target, confirmed: allConfirmed, byExpert: flat };
