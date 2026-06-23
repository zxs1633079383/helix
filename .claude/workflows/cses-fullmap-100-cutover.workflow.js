export const meta = {
  name: 'cses-fullmap-100-cutover',
  description: 'cses 全量纯 helix 化(100% 生产)——Phase1 helix 命令全补 + Phase2 UC单元下放纯渲染 + Phase3 真Go e2e逐UC验证·loop-until-100%',
  whenToUse: 'spec docs/superpowers/specs/2026-06-23-cses-full-helix-cutover-design.md 落地;排除 search + createUser/status',
  phases: [
    { title: 'helix命令全补', detail: 'im_load_older_context + vote5 + 通知待办 + WS缺4handler + dialog/unread投影 + partials收口' },
    { title: 'UC单元下放', detail: '10 UC集合 → 纯渲染(数据/持久/编排/归一化下放),热文件单owner串行' },
    { title: '真Go e2e验收', detail: '逐UC真Go e2e(444+64118eeb)+更新台账+fullmap-coverage闸' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const CW = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ'
const E2E_ENV = 'HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON=\'["678","555"]\' SETTLE_MS=2500'

const CARGO_DISC = `## Cargo 纪律:中途禁 cargo;只 commit 前 cargo check -p <crate> + phase 末 cargo test 各一次。≤2-3 次。`
const HELIX_CI = `## CI 安全:push helix main 前验 git diff -- crates/helix-core/ 为空(api-surface 保绿)+ gate.sh(pre-push 自动跑)。core 零运行时不破。`
const NAMING = `## 命名陷阱保真:camelCase / createAt(非createdAt) / unReadPostId(大写R) / messageType SYSTEN — 后端权威字段名原样,锚 真机curl真源.md + full-map/partials。`
const HONEST = `## 诚实红线:Go物理缺(createUser/status)标 goBlocked 不假装;前端无caller记NEED;绝不假装wiring。`

// ════ Phase 1: helix 命令全补(并行 rust-dev,各独立文件)════
phase('helix命令全补')
const HELIX_GROUPS = [
  { key: 'load-older-context', prompt: `cwd ${HELIX}(main分支)。新增 helix outbound/投影命令 **im_load_older_context** 解上拉翻页缺口(NEED_HELIX_FIX_PULL_REFRESH):legacy im_load_older_messages 有 8轮翻页循环+DB upsert+严格更早过滤(create_at<anchor)+跨批去重+行归一化;helix im_post_context 是单次。实现:① 新命令做翻页编排(driver/module 侧循环 im_post_context 推 anchor 凑够 target 条严格更早)OR 投影命令读本地DB翻页 ② emit 归一化投影 im:messages:older_loaded{channelId,messages[],hasMore}。参 channel_service.rs:468 翻页注释 + full-map UC-2.2。TDD + cargo check。` },
  { key: 'vote-score-5', prompt: `cwd ${HELIX}(main)。helix outbound/vote_score.rs 补 5 个第二网关命令(gateway()==Default,对齐已有 im_vote_create 范式):im_vote_do(vote/vote {id,postId?,indexes:string[]}) / im_vote_close(vote/closeVote {id}) / im_average_attend(average/attend {id,score:number,postId?}) / im_average_close(average/close {id,postId?}) / im_average_publish(average/publish {title,content,maxScore,minScore,isDelMaxMin,isAnonymous,cutoff,members,...})。caller=message-vote/average-content.component。distributed_slice 注册 + TDD + cargo check。` },
  { key: 'notify-todo', prompt: `cwd ${HELIX}(main)。UC-10 通知/待办:full-map 查 messageNotice 的 outbound(待办列表/系统通知端点)→ helix outbound 命令 + 投影。参 full-map/partials/3--backend-http-user-misc + 6--client-usecases 集合十。TDD + cargo check。无 caller 的记 NEED。` },
  { key: 'ws-missing-4', prompt: `cwd ${HELIX}(main)。WS 当前 15/19,补缺 4 个 handler。对照 full-map/partials/5--backend-ws-events 全 19 事件 vs helix ws/handlers/ 已处理,找缺的 4 个补 distributed_slice handler(解析→投影)。守 HX-C010 确定性有序。TDD + cargo check。` },
  { key: 'dialog-unread-projection', prompt: `cwd ${HELIX}(main)。UC-3 已读未读 + 会话列表:把 unread/badge 计算 + dialogList 投影下放 helix(前端 messageDialog 2891行的 unread 计算移除)。新增/扩投影命令吐 im:channels:projection{dialogList[完整DialogItem含unreadCount]}。参 full-map UC-3.x + projection-schema.md。TDD。注意 channel-row 之前留瘦(FixB)——unread 计算下放需 helix 侧算好吐完整,前端零计算。` },
]
const helixResults = await parallel(HELIX_GROUPS.map((g) => () =>
  agent(`${CARGO_DISC}\n${HELIX_CI}\n${NAMING}\n${HONEST}\n\n# helix 命令补全:${g.key}\n${g.prompt}\n\n返回:commit SHA + 命令名 + cargo结果 + 是否守HX-C001 + NEED(如有)。**不push**(主对话统一验+push)。`,
    { label: `helix:${g.key}`, phase: 'helix命令全补', agentType: 'rust-dev', effort: 'high' })
    .then((r) => ({ group: g.key, report: r }))
))

// ════ Phase 2: UC 单元下放纯渲染 ════
// 热文件(message.service/messageDialog/ipcEventHandler)= 多UC共享 → 串行;独立组件文件 → 可并行。
phase('UC单元下放')
const UC_UNITS = [
  { key: 'U1-收发', hot: true, prompt: `UC-1.1~1.10 消息收发。前端 messageSend/message.service/ipcEventHandler 的发送编排(buildMessageObject已下放send_build)、撤回/编辑/转发/快捷回复/加急/定时 → 全改纯 helix invoke + WS回声投影渲染。删本地编排/归一化。` },
  { key: 'U2-历史', hot: true, prompt: `UC-2.1~2.4 历史加载。chatContentShared 上拉翻页循环 → helix im_load_older_context(Phase1已补)。首屏/定位/回复列表走 helix 投影。删前端翻页/去重/归一化。` },
  { key: 'U3-已读', hot: true, prompt: `UC-3.1~3.4 已读未读。message.service/messageDialog 的 unread 计算 → helix 投影(Phase1 dialog-unread-projection 已下放)。前端零 unread 计算,渲染 helix 吐的 unreadCount。` },
  { key: 'U4-同步', hot: false, prompt: `UC-4.1~4.5 离线同步。im-sync/message.service requestChannelIncrement → helix SyncScheduler(已)。前端只触发 + 渲染 increment 投影,删前端 sync 编排。` },
  { key: 'U5-群管理', hot: true, prompt: `UC-5.1~5.7 群管理。message.service 建群/话题/关退/属性/置顶/公告/在线分组 → 纯 helix invoke(channel outbound 已补 permission/messageTop/announcement)。` },
  { key: 'U6-成员', hot: true, prompt: `UC-6.1~6.4 成员管理。拉踢/管理员/昵称/快照 → 纯 helix invoke(channel-member outbound)。` },
  { key: 'U8-投票评分', hot: false, prompt: `UC-8 投票/平均分。message-vote/average-content.component → 纯 helix invoke(vote/score 10命令:已5+Phase1补5)。` },
  { key: 'U9-书签', hot: false, prompt: `UC-9 书签 CRUD → 纯 helix invoke(posts_read outbound 已)。` },
  { key: 'U10-通知待办', hot: false, prompt: `UC-10 通知/待办 → 纯 helix invoke(Phase1 notify-todo 命令)+ 投影渲染。` },
  { key: 'X-DB归一化清除', hot: true, prompt: `横切:全 message-v3 删 store_*/imDataSource 本地DB调用(27文件)+ normalizeMessageFromDB/formatMessageData 归一化 → 全走 helix 投影。前端零本地DB、零归一化。` },
]
// 热文件单 owner 串行(防 message.service/messageDialog 撞);独立的并行
const COMMON2 = `cwd ${CW}(分支 feat/helix-takeover-integ)。**纯 helix 替换**:删 isHelix守卫+legacy分支,helix唯一路径。${NAMING}\n${HONEST}\n域gate:npx tsc --noEmit(本域0新error)+ npx jest(既有绿+新case)。红→诊断修≤3轮→仍红revert本域不污染。绿→单域commit(feat(message-v3/<uc>): 纯helix UC-x 下放)。`
const hotUnits = UC_UNITS.filter((u) => u.hot)
const coldUnits = UC_UNITS.filter((u) => !u.hot)
// 冷单元并行
const coldResults = await parallel(coldUnits.map((u) => () =>
  agent(`${COMMON2}\n\n# 单元 ${u.key}\n${u.prompt}\n完成返回 {unit, gatePassed, commit, needs, jest}。`,
    { label: `uc:${u.key}`, phase: 'UC单元下放', agentType: 'cses-client-dev', effort: 'high' })
    .then((r) => ({ unit: u.key, report: r }))
))
// 热单元串行(共享 message.service/messageDialog)
const hotResults = []
for (const u of hotUnits) {
  const r = await agent(`${COMMON2}\n\n# 单元 ${u.key}(热文件·你是唯一owner)\n${u.prompt}\n完成返回 {unit, gatePassed, commit, needs, jest}。`,
    { label: `uc:${u.key}`, phase: 'UC单元下放', agentType: 'cses-client-dev', effort: 'high' })
  hotResults.push({ unit: u.key, report: r })
  log(`${r?.gatePassed === false ? '🔴' : '✅'} ${u.key}`)
}

// ════ Phase 3: 真 Go e2e 逐 UC 验收 + 台账 + 完整性闸 ════
phase('真Go e2e验收')
const verify = await agent(
  `# Phase 3 全量验收(真 Go e2e·full-map 三表对账)\n` +
  `cwd ${HELIX}。真后端栈已起(Go:8065 + cses java + telepresence)。账号 ${E2E_ENV}。\n` +
  `## 三件:\n` +
  `1. **逐 UC 真 Go e2e**:对 docs/review/uc-coverage-ledger.md 的 37 UC,用 host-cli-go-chain-e2e.sh / host-cli-go-e2e.sh(自建群)逐 UC 跑真 Go,WS 回声 action + 投影字段断言。遇 gRPC+"unknown" → /Users/mac28/workspace/java/zlc_ai/GenericAgent/tp-connect.sh + 重启 /workspace/java/cses。**跑绿的 UC 在台账标 ✅ e2e-verified**(Edit ledger)。\n` +
  `2. **full-map 三表对账**:bash scripts/fullmap-coverage.sh + 人工核 84HTTP/19WS/37UC 覆盖,列未覆盖项。\n` +
  `3. **对抗 review**:git diff 逐 UC commit——转纯真删legacy / endpoint·body对齐full-map/真机curl / 命名陷阱 / 无假wiring / helix-core零运行时不破。\n` +
  `产出:**完整性矩阵**(HTTP/WS/UC 各 x/total)+ **DONE判定**(三表100%?)+ 每UC e2e PASS/FAIL/goBlocked + 未覆盖项清单(下一轮 resumeFromRunId 续)+ review findings。`,
  { label: 'e2e-verify+对账', phase: '真Go e2e验收', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
)

return {
  helix: helixResults.filter(Boolean).map((r) => ({ group: r.group, report: String(r.report).slice(0, 300) })),
  ucCold: coldResults.filter(Boolean).map((r) => ({ unit: r.unit, report: String(r.report).slice(0, 200) })),
  ucHot: hotResults.map((r) => ({ unit: r.unit, report: String(r.report).slice(0, 200) })),
  verify: String(verify),
}
