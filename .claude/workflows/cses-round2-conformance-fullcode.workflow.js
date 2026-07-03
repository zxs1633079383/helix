export const meta = {
  name: 'cses-round2-conformance-fullcode',
  description: 'Round-2:全代码铁律(禁gap-only/骨架/纯文档)+ 逐UC双闸(契约审计+行为e2e)。补首轮NEED为真实现 + 35UC完整下放',
  whenToUse: 'spec docs/superpowers/specs/2026-06-24-cses-round2-conformance-fullcode-design.md;排除 search+createUser/status',
  phases: [
    { title: 'helix缺命令全补', detail: 'todo装配链/sync-increment/usermisc-write/HTTP partials → 真outbound命令(禁NEED/骨架)' },
    { title: 'cses下放全代码', detail: '35UC真接线+契约审计(重审首轮gap-only档:U4同步/U5群管/U10通知)' },
    { title: '双闸验收', detail: '逐UC契约审计(always)+行为e2e(有数据态)→台账三态' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const CW = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ'
const E2E = 'HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON=\'["678","555"]\' SETTLE_MS=2500'

// 🔴 全代码铁律(注入每个实现 agent)
const FULLCODE = `
## 🔴 全代码铁律(最高优先·硬禁)
- **禁** \`todo!()\`/\`unimplemented!()\`/空骨架占位。
- **禁**「前端零caller故跳过」「已做无需改」「这是gap记NEED」这类**自以为是的跳过**——除非你**契约审计证明**(贴 full-map UC 规格 + 现状 diff)确实无需改,否则**必须写完整可工作代码**。
- **禁**纯 docs/SESSION 回填交差当"完成"。
- helix 缺命令 → **真实现 outbound 命令**(distributed_slice 注册 + build 返 endpoint+body),不是写 NEED 文件。
- cses 下放 → **真接线**(isHelix 守卫删 + helix invoke/投影 渲染完整路径),不是结论。
- 唯一可不实现:search(集合七)+ createUser/status(Go物理缺handler)——其余全代码。
- 交付必须有**实质代码 diff** 或 **契约审计证据**(证明现状已符合UC)。
`
const CONTRACT = `
## 契约审计闸(always)
逐字对齐 full-map/partials/<域>.md UC 规格:① 端点(posts/create 非 createPosts 等真路径)② body 字段集(vs 真机curl真源.md)③ 命名陷阱(camelCase/createAt非createdAt/unReadPostId大写R/messageType SYSTEN)。出偏差→修。
`
const CARGO = `## Cargo纪律:中途禁cargo;commit前 cargo check -p<crate>+cargo test 各一次。守HX-C001(core零运行时)。`
const CI = `## CI安全:push helix main前 git diff -- crates/helix-core/ 空(api-surface绿)。`

// ════ Phase 1: helix 缺命令全补(真实现,禁NEED)════
phase('helix缺命令全补')
const HELIX_GAPS = [
  { key: 'todo-assembly', prompt: `UC-10 通知/待办**完整实现**(首轮只记NEED_HELIX_TODO,本轮真做)。helix-im 补 todo 装配链:hello收尾触发 im_query_todo_list(查 outbound/posts_read.rs:145 是否已注册,补编排) + 待办投影 emit + 系统通知。对照 full-map/partials/3 + 6集合十 + 真机curl真源。TDD。` },
  { key: 'sync-increment', prompt: `UC-4 离线同步**完整实现**(首轮记NEED_HELIX_FIX_SYNC_INCREMENT)。补 helix 增量 sync outbound/编排缺口(requestChannelIncrement 链)。对照 full-map UC-4.1~4.5 + 02-迁移蓝图 increment。TDD。` },
  { key: 'usermisc-write', prompt: `user-misc 写族**完整实现**(首轮记NEED_USERMISC_WRITE_FAMILY)。announcement写已补;其余 user-misc/team 写命令真实现 outbound(即便前端暂无caller也实现命令,禁跳过)。排除 createUser/status(goBlocked)。对照 full-map/partials/3。TDD。` },
  { key: 'http-partials', prompt: `补全 HTTP outbound 命令到全(fullmap-coverage 显 39/79)。逐条扫 full-map/partials/1-3 的端点 vs helix 已注册 outbound,**缺的全补真命令**(排除 search/createUser/status)。每条 wire 对齐真机curl真源。TDD。` },
]
const helixR = await parallel(HELIX_GAPS.map((g) => () =>
  agent(`${FULLCODE}\n${CONTRACT}\n${CARGO}\n${CI}\n\ncwd ${HELIX}(main)。# ${g.key}\n${g.prompt}\n返回 commit SHA+命令名+cargo+实质代码证据(禁NEED/骨架)。不push。`,
    { label: `helix:${g.key}`, phase: 'helix缺命令全补', agentType: 'rust-dev', effort: 'high' }).then((r) => ({ g: g.key, r }))))

// ════ Phase 2: cses 下放全代码(禁gap-only,重审首轮"查清"档)════
phase('cses下放全代码')
const UC_COLS = [
  { key: 'U4-同步-重审', prompt: `UC-4.1~4.5。首轮 d74cd3e31 是"查清gap-only"(92行全docs)→ **本轮真审+真接线**:前端 im-sync/message.service 增量触发链是否真纯helix?触发→渲染increment投影 完整路径有没有真代码?缺即实现,不是结论。` },
  { key: 'U5-群管理-重审', prompt: `UC-5.1~5.7。首轮 8375b4254 是"6行验证no-op"→ **本轮契约审计证明**:建群/话题/关退/属性/置顶/公告/在线 7 项每项 helix invoke 真接 + 契约对齐?若真已全转纯,贴逐UC契约审计证据;若有缺(如在线分组UC-5.7)即实现。` },
  { key: 'U10-通知待办-重审', prompt: `UC-10.1~10.2。首轮 b05736e20 是"零caller结论"→ **本轮真接线**:待办列表/系统通知 前端渲染 helix todo 投影(Phase1 todo-assembly 命令)完整路径,真代码。` },
  { key: 'U6-成员-补', prompt: `UC-6.1~6.4 拉踢/管理员/昵称/快照。首轮 U6 报"clean tree无改"→ 审是否真全转纯+契约对;缺即实现。` },
  { key: 'U3-已读-补', prompt: `UC-3.1~3.4。首轮 b4a93cae5 "onChannelRead gap-only"→ 真接 onChannelRead + 未读数完整渲染 helix 投影。` },
]
const COMMON2 = `cwd ${CW}(分支 feat/helix-takeover-integ)。纯helix(删isHelix守卫)。${FULLCODE}\n${CONTRACT}\n域gate:tsc+jest绿才commit;红→修≤3轮→仍红revert。`
const csesR = []
for (const u of UC_COLS) {  // 串行(共享message.service/messageDialog)
  const r = await agent(`${COMMON2}\n\n# ${u.key}\n${u.prompt}\n返回{unit,gatePassed,commit,实质代码或契约审计证据,jest}。禁gap-only/纯docs交差。`,
    { label: `uc:${u.key}`, phase: 'cses下放全代码', agentType: 'cses-client-dev', effort: 'high' })
  csesR.push({ u: u.key, r }); log(`${r?.gatePassed === false ? '🔴' : '✅'} ${u.key}`)
}

// ════ Phase 3: 双闸验收 ════
phase('双闸验收')
const verify = await agent(
  `# Round-2 双闸验收(契约审计 + 行为e2e)cwd ${HELIX}。真后端栈已起。账号 ${E2E}。\n` +
  `## 逐 UC 走双闸(docs/review/uc-coverage-ledger.md 的 37 UC,排除search 2):\n` +
  `**闸1 契约审计(always)**:每UC 比 full-map/partials 规格——端点/body字段集(vs真机curl真源)/命名陷阱。出偏差列出。\n` +
  `**闸2 行为e2e(有数据态)**:scripts/host-cli-FULL-uc-e2e.sh 跑全量，必要时用 scripts/host-cli-session.sh 自建群补 UC，WS回声+投影断言。遇gRPC+unknown→/Users/mac28/workspace/java/zlc_ai/GenericAgent/tp-connect.sh+重启/workspace/java/cses。\n` +
  `## 台账三态(Edit ledger)**禁gap-only态**:\n` +
  `- ✅ conformant-verified(闸1+闸2全过) · ⏸ code-complete(全代码+闸1过,e2e缺数据) · 🔴 goBlocked(仅search/createUser/status)\n` +
  `## 验本轮 helix/cses commit 有无骨架/gap-only/纯docs偷懒(git diff 审):有 todo!()/空实现/纯结论→列为 finding 退回。\n` +
  `产出:37UC三态矩阵 + 偏差清单 + 偷懒finding + fullmap-coverage三表 + DONE判定(全 code-complete+可验verified?)。`,
  { label: '双闸验收', phase: '双闸验收', agentType: 'rust-tauri-skeleton-dev', effort: 'high' })

return { helix: helixR.filter(Boolean).map((x) => ({ g: x.g, r: String(x.r).slice(0, 250) })), cses: csesR.map((x) => ({ u: x.u, r: String(x.r).slice(0, 200) })), verify: String(verify) }
