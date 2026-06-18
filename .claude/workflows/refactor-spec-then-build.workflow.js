export const meta = {
  name: 'refactor-spec-then-build',
  description: '对给定重构项：先出 spec(md)+mermaid(mmd) → 专家对抗审 → 串行实现(自跑 cargo check/test) → 终验。过程全留痕到 docs/refactor。',
  phases: [
    { title: 'Spec', detail: '每个重构项出设计 spec + mermaid before/after' },
    { title: 'Review', detail: 'rust-dev 对抗审 spec：不破 HX-C/GOAL、enum 设计成立' },
    { title: 'Implement', detail: '串行实现(B→A，都改 module.rs)；自跑 cargo check + test -p helix-im' },
    { title: 'Verify', detail: '双端 cargo check + helix-im 全测 + grep 闸门' },
  ],
}

// ---- args shim（README 约束：args 以 JSON 字符串注入，必须归一化）----
const ARGS = (typeof args === 'string' ? (args ? JSON.parse(args) : {}) : (args || {}))

const DOCS = 'crates/helix-im/docs/refactor'

// 两个重构项（B 先 A 后：都改 module.rs，必须串行；B 风险低先落）
const REFACTORS = ARGS.refactors || [
  {
    id: 'B',
    slug: 'corr-map-unify',
    title: 'B · 统一 corr 回报路由表（5 张 HashMap → 单 corr_map + CorrelationContext enum）',
    现状: 'module 里 corr_to_sync / corr_to_channel / corr_to_send / scan_corr / continuation_* 多张 HashMap 各管一类 HTTP 回报；handle_port_reply 150 行深 if-let；加一类 HTTP 要散改 3 处。',
    目标: '合并成单一 state.corr_map: HashMap<Correlation, CorrelationContext>，CorrelationContext 为 tagged enum(Sync/Channel/Send/Scan/Continuation/...)；handle_port_reply 收口为一个 match。加新 HTTP = 加一个枚举臂 + 一个 match 臂。',
    主改文件: ['crates/helix-im/src/state.rs', 'crates/helix-im/src/module.rs（handle_port_reply + 各 insert 点）', 'crates/helix-im/src/acl/to_effect.rs（如涉及）'],
  },
  {
    id: 'A',
    slug: 'inbound-event-unify',
    title: 'A · 合并二重分类（FrameClass + EventKind → 单 InboundEvent enum）',
    现状: 'parser.rs 的 FrameClass(顶层 action 分类) 与 sync_session.rs 的 EventKind(type 数字) 二重分类，加一种消息要改两个 enum + 多个解析函数；module 两层 match 套娃。',
    目标: '合并成单一 InboundEvent enum，一次解析分到底，module.handle 一层 match。保持 parser 边界零 panic、零信任不变。',
    主改文件: ['crates/helix-im/src/parser.rs', 'crates/helix-im/src/sync_session.rs', 'crates/helix-im/src/module.rs（handle Tick::Inbound 分支）'],
  },
]

const SPEC_SCHEMA = {
  type: 'object',
  required: ['specPath', 'mmdPath', 'summary', 'changePoints'],
  properties: {
    specPath: { type: 'string', description: '写出的 spec.md 绝对/相对路径' },
    mmdPath: { type: 'string', description: '写出的 .mmd 路径' },
    summary: { type: 'string', description: '设计要点一段话' },
    changePoints: { type: 'array', items: { type: 'string' }, description: '逐文件改动点(file:行/函数)' },
    invariantsKept: { type: 'string', description: '如何不破 HX-C001..008 / GOAL §1' },
  },
  additionalProperties: true,
}

const REVIEW_SCHEMA = {
  type: 'object',
  required: ['verdict', 'mustFix'],
  properties: {
    verdict: { type: 'string', enum: ['sound', 'sound-with-fixes', 'reject'] },
    mustFix: { type: 'array', items: { type: 'string' } },
    risks: { type: 'array', items: { type: 'string' } },
  },
  additionalProperties: true,
}

const IMPL_SCHEMA = {
  type: 'object',
  required: ['filesChanged', 'checkPass', 'testPass'],
  properties: {
    filesChanged: { type: 'array', items: { type: 'string' } },
    checkPass: { type: 'boolean' },
    testPass: { type: 'boolean' },
    testSummary: { type: 'string', description: 'cargo test -p helix-im 结果(通过数/失败项)' },
    notes: { type: 'string' },
    blockers: { type: 'array', items: { type: 'string' } },
  },
  additionalProperties: true,
}

const VERIFY_SCHEMA = {
  type: 'object',
  required: ['pass'],
  properties: {
    pass: { type: 'boolean' },
    checkResult: { type: 'string' },
    testResult: { type: 'string' },
    gateResult: { type: 'string' },
    remaining: { type: 'array', items: { type: 'string' } },
  },
  additionalProperties: true,
}

const COMMON = `
本仓 = helix（runtime-agnostic 确定性内核）。铁律：
- helix-im 只依赖 helix-core，禁 tokio/sqlx/reqwest（HX-C001）。
- handle 是同步纯函数，副作用只经 out.push(Effect::..)；外部输入零 unwrap/expect(HX 边界零信任)。
- 不改 helix-core 的 Effect/Tick 变体（本次重构纯 helix-im 内部）。
- 行为真源：与现网 cses-client im 行为有争议时以现网为准；保持回放测试全绿。
`

// ===== Phase 1: Spec（并行，输出文件不同，安全）=====
phase('Spec')
const specs = await parallel(REFACTORS.map((r) => () =>
  agent(
    `${COMMON}
你的任务：为重构项【${r.title}】产出**设计 spec + mermaid 图**，不写实现代码。

先 Read 真实源码（${r.主改文件.join(' / ')}）摸清现状，再产出：
1. Write 一份 spec 到 \`${DOCS}/${r.id}-${r.slug}-spec.md\`，结构：
   ## 背景(现状痛点) / ## 目标设计(新 enum 定义草案，贴 Rust 类型) / ## 逐文件改动点(file:函数，精确)
   / ## 不变量保持(怎么不破 HX-C001..008 + handle 同步纯函数 + 边界零 panic) / ## 回放测试影响(哪些测试需调，断言不变)
   / ## 风险与回滚
2. Write 一份 mermaid 到 \`${DOCS}/${r.id}-${r.slug}.mmd\`：用 flowchart 画**改前 vs 改后**的数据流/分类路径对比（两个 subgraph：Before / After）。
   同时把这段 mermaid 也嵌进 spec.md 的「目标设计」节(\`\`\`mermaid 围栏)。
现状：${r.现状}
目标：${r.目标}

要求：enum 定义要具体可编译(字段/变体齐全)；改动点精确到函数名；不臆造行号(用函数名锚定)。返回结构化结果。`,
    { schema: SPEC_SCHEMA, label: `spec:${r.id}`, phase: 'Spec', agentType: 'rust-dev' }
  ).then((s) => ({ r, spec: s }))
))
const okSpecs = specs.filter(Boolean)
log(`Spec 完成：${okSpecs.map((x) => x.r.id).join(',')}`)

// ===== Phase 2: Review（并行，对抗审 spec）=====
phase('Review')
const reviewed = await parallel(okSpecs.map(({ r, spec }) => () =>
  agent(
    `${COMMON}
你是对抗审查者。Read spec \`${spec.specPath}\` 与相关真实源码，**默认怀疑**地审重构项【${r.title}】的设计：
- enum 设计是否真能覆盖现状所有分支(别漏 case)？合并后是否引入歧义？
- 是否破坏 HX-C001(零运行时)/HX-C002(Send/生命周期)/HX-C005(热路径 O(1)) 或 handle 同步纯函数？
- 边界解析是否仍零 panic？回放测试断言是否会被迫改语义(语义变=危险)？
- 改动点是否真覆盖所有调用方(grep 验证 corr_to_* / FrameClass / EventKind 的所有引用)？
给 verdict(sound/sound-with-fixes/reject) + mustFix[](实现前必须修正的设计漏洞) + risks[]。证据要具体。`,
    { schema: REVIEW_SCHEMA, label: `review:${r.id}`, phase: 'Review', agentType: 'rust-dev' }
  ).then((rev) => ({ r, spec, rev }))
))
const okReviewed = reviewed.filter(Boolean)
for (const x of okReviewed) log(`Review ${x.r.id}: ${x.rev.verdict}${x.rev.mustFix?.length ? ' / mustFix:' + x.rev.mustFix.length : ''}`)

// reject 的不进实现
const buildable = okReviewed.filter((x) => x.rev.verdict !== 'reject')
// 串行顺序：B 先 A 后（都改 module.rs）
buildable.sort((a, b) => (a.r.id === 'B' ? -1 : 1))

// ===== Phase 3: Implement（串行！都改 module.rs，禁并行）=====
phase('Implement')
const impls = []
for (const { r, spec, rev } of buildable) {
  const res = await agent(
    `${COMMON}
按 spec 实现重构项【${r.title}】。
- Read spec \`${spec.specPath}\` + review mustFix：${JSON.stringify(rev.mustFix || [])} —— 实现时必须吸收这些修正。
- Read 当前真实源码（注意：可能已有上一个重构项的改动，以**磁盘现状**为准，不要覆盖）。
- 严格按 spec 改动点实现；保持对外行为不变（纯内部重构）。
- 改完**自跑验证**(autonomous 模式)：\`cargo check -p helix-im\` 必须过；\`cargo test -p helix-im\` 必须全绿(45+ 回放测试)。
  - 测试若红：优先修实现(不是改测试语义)；确实是测试需随新类型调整的，只改构造不改断言语义。
- 禁改 helix-core；禁引入 tokio/sqlx/reqwest；parser 保持零 unwrap。
返回 filesChanged[] + checkPass + testPass + testSummary + blockers[](若有过不去的)。`,
    { schema: IMPL_SCHEMA, label: `impl:${r.id}`, phase: 'Implement', agentType: 'rust-dev' }
  )
  impls.push({ r, res })
  log(`Impl ${r.id}: check=${res?.checkPass} test=${res?.testPass}${res?.blockers?.length ? ' blockers:' + res.blockers.length : ''}`)
}

// ===== Phase 4: Verify（终验，双端 + 闸门）=====
phase('Verify')
const verify = await agent(
  `${COMMON}
终验 A+B 重构落地。在仓库根跑并报告(autonomous 自跑)：
1. \`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi\` —— 双端编译全绿？
2. \`cargo test -p helix-im\` —— 回放测试全绿？(贴通过/失败数)
3. 闸门：\`cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'\` 必须空；\`grep -rE 'unwrap|expect|panic' crates/helix-im/src/parser.rs\` 应≈0；\`bash scripts/gate.sh\`(若适用)。
返回 pass(全绿才 true) + checkResult + testResult + gateResult + remaining[](遗留)。`,
  { schema: VERIFY_SCHEMA, label: 'verify', phase: 'Verify', agentType: 'rust-dev' }
)
log(`Verify: pass=${verify?.pass}`)

return {
  specs: okSpecs.map((x) => ({ id: x.r.id, spec: x.spec.specPath, mmd: x.spec.mmdPath })),
  reviews: okReviewed.map((x) => ({ id: x.r.id, verdict: x.rev.verdict, mustFix: x.rev.mustFix })),
  impls: impls.map((x) => ({ id: x.r.id, ...x.res })),
  verify,
}
