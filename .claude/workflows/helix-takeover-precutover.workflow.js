export const meta = {
  name: 'helix-takeover-precutover',
  description: 'cutover 前置 — PS 冻结 projection-schema(C2) + CP createPosts 补全 88→89(D3)',
  phases: [{ title: 'PreCutover', detail: 'PS projection-schema 冻结+强制测试 / CP createPosts 89' }],
}

const WT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/im-takeover'
const DOCS = `${WT}/crates/helix-im/docs/takeover-plan`
const CLIENT = '/Users/mac28/workspace/angular/cses-client'
const MM = '/Users/mac28/workspace/golangProject/mattermost'

const COMMON =
  `你在集成 worktree 做 helix-im 接管「cutover 前置」的一个任务。\n` +
  `工作区（绝对路径）：${WT}　分支：feat/im-takeover\n\n` +
  `## 背景\n` +
  `helix-im 已全权接管 cses-client HTTP+WS+业务（P0-P7 落地，88 outbound + 19 WS + Tauri 四接缝 + im:__bus__ 单总线）。` +
  `本任务清掉 cutover 前最后 2 个前置缺口。先 Read：${DOCS}/01-接管架构与状态机.md（IPC 兼容两表 + 投影 payload）+ ${DOCS}/04-cses-client接入方案.md（真实 IPC 面收口）。\n\n` +
  `## 铁律\n` +
  `- 【HX-C001】helix-core 零运行时；业务放 helix-im；token 留 driver。【确定性 C3】helix-im/src grep SystemTime|Instant::now|Utc::now 必空。\n` +
  `- 【bitmap 禁区】不碰 bitmap/segment。【行为真源】现网 ${CLIENT}/src/pages/message-v3/（前端）+ src-tauri/features/im/ 为准；真机curl真源.md。\n` +
  `- 【测试】配测试类，新文件 100% 行覆盖（cfg(test) 漏行可排除）。\n` +
  `- 【编译纪律】中途 rust-analyzer 自审，禁 cargo check 循环；仅完成前跑一次 gate（--all-targets 全树）。\n` +
  `- 【git/HX-C013】只在 ${WT} 内 git add+commit 到 feat/im-takeover；禁 checkout/branch/merge/push；提交前自检 pwd+分支。\n` +
  `- 【命令】cargo test --manifest-path ${WT}/Cargo.toml -p <crate>；cargo check --manifest-path ${WT}/Cargo.toml -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-host-cli --all-targets；` +
  `cargo tree --manifest-path ${WT}/Cargo.toml -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'（空）；bash ${WT}/scripts/structure-gate.sh。\n` +
  `- 【提交体】§6 结构化 5 段。\n` +
  `- 【结构闸纪律】**不要为塞代码上调 structure-gate baseline**（drift-review 反复点名的棘轮后门）——超 300 就拆文件，宁可 mod.rs 多一个子文件。\n\n` +
  `## 完成判据\n` +
  `gate 全绿（cargo test + check --all-targets + core 零运行时/确定性 grep 空 + 新文件 100% 覆盖 + structure-gate PASS + 本任务专属断言）才算完成；红则如实 gatePassed:false + 卡点。\n`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' }, gatePassed: { type: 'boolean' }, commit: { type: 'string' },
    summary: { type: 'string' }, gateOutput: { type: 'string' }, coverage: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' } },
    findings: { type: 'string', description: '关键发现（如 createPosts 真实 casing 实证、前端是否真有 caller、DialogItem 字段集差异）' },
  },
}

const PHASES = [
  { id: 'PS', title: '冻结 projection-schema.md（C2）+ 强制对齐测试', agentType: 'rust-tauri-skeleton-dev', detail:
    `目标：把 helix 经 im:__bus__ emit 的**投影 payload 字段集冻结成真源文档 + 强制测试**，让"前端纯渲染"可证伪。\n` +
    `步骤：\n` +
    `1. 枚举 helix 全部投影 emit 出口：grep ${WT}/crates/helix-im/src 的 to_bus_envelope / Effect::Emit / DomainEvent 构造 + ${WT}/crates/helix-driver-native 的 bus_guard/event_sink emit 点；列出每个 im:__bus__ channel（im:post:received / im:channel:update / im:messages:query_result / im:sync:* 等）的 payload 字段集。\n` +
    `2. 读现网前端契约：${CLIENT}/src/pages/message-v3/ 的 DialogItem interface + Message/Post 渲染所需字段（readBits/unReadPostId/lastPost/messageType 等，参 03/01 文档「迁移服务端字段契约清单 10 项」+ 07-client-ui-rendering 报告）。\n` +
    `3. 写 ${WT}/crates/helix-im/docs/projection-schema.md：逐 channel 冻结 payload 字段（名/类型/来源/对应前端 DialogItem 字段），标注命名陷阱（unReadPostId 大写R / messageType SYSTEN 拼写 / createAt）。这是 cutover 契约真源。\n` +
    `4. **强制测试**（C2 可证伪核心）：新 tests/projection_schema_test.rs——构造触发各 emit 的 Tick，断言 emit 出的 payload 字段集 == projection-schema.md 冻结集（字段缺失/多余即 fail）；至少覆盖 im:post:received + im:channel:update + 投影命令 result。字段不齐 = 前端要补算 = 纯渲染破功 = 测试红。\n` +
    `gate 额外：projection-schema.md 与测试断言的字段集一致；测试 100% 覆盖新增断言路径。` },
  { id: 'CP', title: 'createPosts 补全 88→89（D3 真实 body 实证）', detail:
    `目标：补齐唯一挂起的 outbound 命令 createPosts(posts #2)，outbound 总数 88→89。\n` +
    `步骤：\n` +
    `1. **实证真实 body**（非臆断 casing）：读前端真实调用点 ${CLIENT}/src/pages/message-v3/ 里 createPosts/批量建帖的 service 方法——前端构造的 body 就是上线的真实 wire（等价抓包）；同时读后端 ${MM}/server/channels/csesapi/ 的 createPosts handler 解构的 struct（确认 json tag：有 tag 按 tag / 无 tag Go 大小写不敏感匹配 PascalCase 字段）。交叉判定真实 casing（Posts/ChannelIds PascalCase vs posts/channelIds camelCase）。findings 里写清证据链 + 若前端无 caller 据后端 struct 定 + 标注「frontend-source-derived 非 live 抓包」。\n` +
    `2. 实现 im_create_posts outbound 命令：落 ${WT}/crates/helix-im/src/outbound/（按 structure-gate ≤300 选既有 posts 写族文件追加或新文件 + distributed_slice 注册）；body 按步骤1 实证的真实 casing 拼装。\n` +
    `3. 测试类 tests/cp_create_posts_test.rs：正常路径 + **casing fail-hard 断言**（按实证：若 PascalCase 则断言 body 含 "Posts"/"ChannelIds" 不含 camelCase，反之亦然）+ 真 Go fixture 比对。\n` +
    `gate 额外：outbound 注册总数从 88→89（性质测试断言注册数）；createPosts body casing 断言锚实证结论。` },
]

phase('PreCutover')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红，跳过' }); continue }
  const opts = { label: `pre-${p.id}`, phase: 'PreCutover', schema: SCHEMA, effort: 'high' }
  if (p.agentType) opts.agentType = p.agentType
  const r = await agent(
    `${COMMON}\n\n## 你的任务 = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程：Read 规格/真源 → 实现 → 跑完整 gate(--all-targets) → gate 绿则结构化提交 → 返回结构化结果。`,
    opts)
  results.push(r)
  if (!r || !r.gatePassed) { stop = true; log(`⛔ ${p.id} gate 未过 → 停`) }
  else log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}`)
}
return { results, stoppedEarly: stop, branch: 'feat/im-takeover', worktree: WT }
