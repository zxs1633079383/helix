export const meta = {
  name: 'helix-takeover-impl-g2p6p7',
  description: 'helix-im 接管实现收官 — P5a user-misc → P5b bot-agent → P6 Tauri 四接缝 → P7 cses-client 接入设计（只设计）',
  phases: [{ title: 'G2+P6+P7', detail: 'P5a user-misc / P5b bot-agent / P6 Tauri 接缝(M2) / P7 接入设计' }],
}

const WT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/im-takeover'
const DOCS = `${WT}/crates/helix-im/docs/takeover-plan`

const COMMON =
  `你在集成 worktree 实现 helix-im「全权接管 cses-client HTTP+WS+业务」方案的一个 Phase。\n` +
  `工作区（绝对路径）：${WT}　分支：feat/im-takeover\n\n` +
  `## 开工先 Read 规格\n` +
  `- ${DOCS}/02-分阶段路线.md（你这个 Phase 小节）+ ${DOCS}/03-测试策略.md（测试矩阵，每接口/事件配测试类 100% 覆盖）+ ${DOCS}/01-接管架构与状态机.md（架构边界）。\n` +
  `- P7 另读 ${DOCS}/04-cses-client接入方案.md（草案，你要按 P0-P6 真实落地的 IPC 面 finalize）。\n` +
  `- 行为真源：/Users/mac28/workspace/angular/cses-client/（前端 message-v3 + src-tauri/features/im）；` +
  `${WT}/crates/helix-im/docs/csesapi-migration/真机curl真源.md；后端 /Users/mac28/workspace/golangProject/mattermost/server/channels/csesapi/。\n\n` +
  `## 铁律\n` +
  `- 【HX-C001】helix-core 零运行时；业务放 helix-im；鉴权 token 注入留 driver（grep mp_/Authorization in helix-im/src 必空）。\n` +
  `- 【确定性 C3】时间 Clock::now_ms、id IdSource::next_id 或前端透传；helix-im/src grep SystemTime|Instant::now|Utc::now 必空（host/driver 是 ambient 合法落点，不在此限）。\n` +
  `- 【bitmap 禁区】不碰 bitmap/segment/channel_bitmap。\n` +
  `- 【热路径 HX-C005】写走字段级 BatchUpdate WHERE PK 禁 RMW。【HX-C007】driver 批处理 Vec<Effect> 不逐条。【HX-C010】迭代→有序 Effect 前 sort。\n` +
  `- 【测试】每接口/事件配测试类（回放剧本 ${WT}/crates/helix-im/tests/common.rs）；真 Go fixture 比对不自造（C5）。\n` +
  `- 【编译纪律】中途 rust-analyzer 自审，禁 cargo check 循环；仅 Phase 完成前跑一次 gate。\n` +
  `- 【git/HX-C013】只在 ${WT} 内 git add+commit 到 feat/im-takeover；禁 checkout/branch/merge/push；提交前自检 pwd+分支。\n` +
  `- 【命令】cargo test --manifest-path ${WT}/Cargo.toml -p <crate> ；` +
  `**gate 必跑 --all-targets 全树编译**（教训：P1 漏 ffi 测试 fixture，per-crate check 漏掉，--all-targets 才命中）：` +
  `cargo check --manifest-path ${WT}/Cargo.toml -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-host-cli --all-targets ；` +
  `cargo tree --manifest-path ${WT}/Cargo.toml -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'（必空）；覆盖率 ${WT}/scripts/coverage.sh --phase <新文件>；结构闸 bash ${WT}/scripts/structure-gate.sh。\n` +
  `- 【提交体】§6 结构化 5 段，scope 标 Phase 号。\n\n` +
  `## 完成判据\n` +
  `gate 全绿才完成 = cargo test 过 + cargo check --all-targets 全树过 + core 零运行时/确定性 grep 空 + 文档列 fail-hard 断言过 + 新文件 100% 行覆盖（cfg(test) 块漏行可排除）+ structure-gate PASS。\n` +
  `gate 红绝不硬提交假绿：如实返回 gatePassed:false + 卡点。拿不准的边界按行为真源实证。\n`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' }, gatePassed: { type: 'boolean' }, commit: { type: 'string' },
    summary: { type: 'string' }, gateOutput: { type: 'string' }, coverage: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' } },
  },
}

const PHASES = [
  { id: 'P5a', title: 'user-misc 域 outbound', detail:
    'users/list、teams(upsert/member add/quit)、modules/getAll、notification/loadSend、health、search 四件套、personal_guard。新 src/outbound/user_misc.rs（distributed_slice 注册，与 P5b bot_agent.rs 零交集）。' +
    'wire：修源 4 bug(double-decode/无响应体/offset Page*Size/message 笔误)；PERSONAL_USER_FORBIDDEN verbatim；search data 以源裸数组为真源 + 补 channelDetails 第四 key + 去 map 套 key；health 裸 {"status":"OK"} 不走信封特例。' +
    '**含 presence（开放决策 D2 已拍：helix 自建）**：users/status/ids 批量在线状态接管为接管完整性前置（纯渲染必需）。完整规格见 02 文档 P5a + 03。' },
  { id: 'P5b', title: 'bot-agent 域 outbound', detail:
    'createBot/botCallback/bot CRUD/bot-agent config/调用(mp_ token)/agent 会话/webhook config。新 src/outbound/bot_agent.rs（distributed_slice 注册）。' +
    'wire：mp_ token 鉴权意图标 X-Auth-Kind:bot，实际 token 注入留 driver（grep mp_ in core 必空 C6）；#2/#32 内存构造 post 不推 cursor；#35-37 响应裸 map 非 CommonRes 特例；timeline/cost(#33/34) 暂不接管(开放决策 D7 废弃)。完整规格见 02 文档 P5b + 03。' },
  { id: 'P6', title: 'Tauri 四接缝接入（M2，driver-native）', agentType: 'rust-tauri-skeleton-dev', detail:
    '现网对齐唯一权威核验 + 落地 driver-native 四接缝：① 真 app.emit("im:__bus__") 信封（channel 必 im: 开头且非 im:__，grep 守护防自环；数组 payload 须 {items:[]} 包装）② invoke 路由(语义命令→Tick::Command 薄壳全集 + 投影命令 im_query_messages_by_channel 复用 store_get_messages 语义 + 消除 imDataSource:* 3 个 early-return 拦截块)③ store:* 写收归 Rust ④ 批量解释 Vec<Effect>(HX-C007)。补 im_get_state/im_delete_all_dialogs。' +
    '**并解决两个遗留 blocker**：(a) userSnapshot→ucenter 注入——host 装配层(host-cli 参照 + Tauri 壳)把登录态展开成 ImConfig.user_name/org_name/dept_name 注入(P1 占位转真值)；(b) 激活 P1.5 出站横切层——host-cli 改 NativeHttp::with_crosscut 接横切(鉴权/401/断网/trace)，消除 drift-review finding① 的"装配就绪未接泵"。' +
    'host-cli 是 Tauri src-tauri 的 headless 同构孪生，以它为参照实现 + 文档化 Tauri 壳的等价接法。完整规格见 02 文档 P6 + 01 架构 IPC 兼容两表。createPosts PascalCase 仍挂 blocker（待真抓包 D3）。' },
  { id: 'P7', title: 'cses-client 无缝接入方案（本轮只设计不写码）', detail:
    '**纯设计，不碰 cses-client 任何代码**。按 P0-P6 真实落地的 IPC 面（invoke 命令全集 + im:__bus__ 事件全集 + 投影命令）finalize ${DOCS}/04-cses-client接入方案.md：' +
    '① 二进制替换 src-tauri/features/im 的接法 ② IPC 兼容层/灰度开关(env IM_ENGINE + 前端 service legacy feature flag 双写分支，D4) ③ 前端纯渲染改造清单(逐 service 标删业务/保渲染) ④ 回滚策略 ⑤ 联调验收 checklist。' +
    '对照真实实现的命令/事件清单 vs 现网 message-v3 service 调用面，列出每个兼容缺口 + 接法。gate = 文档自洽(命令/事件清单与 P0-P6 实际注册的一致) + cargo test 全绿(不引入代码改动) + structure-gate。' },
]

phase('G2+P6+P7')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红，跳过' }); continue }
  const opts = { label: `impl-${p.id}`, phase: 'G2+P6+P7', schema: SCHEMA, effort: 'high' }
  if (p.agentType) opts.agentType = p.agentType
  const r = await agent(
    `${COMMON}\n\n## 你的 Phase = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程：Read 规格 → 实现 → 跑完整 gate(--all-targets) → gate 绿则结构化提交 → 返回结构化结果。`,
    opts)
  results.push(r)
  if (!r || !r.gatePassed) { stop = true; log(`⛔ ${p.id} gate 未过 → 停，状态机不转移`) }
  else log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}（cov: ${r.coverage || 'n/a'}）`)
}
return { results, stoppedEarly: stop, branch: 'feat/im-takeover', worktree: WT }
