export const meta = {
  name: 'cses-round7-postmerge-reverify',
  description: 'merge(e869a0d) 后全量 e2e 重验三表(确认 codex 结构重构没破行为) + 重跑 4.3/2.3/1.5。已有 e2e 脚本逐簇真 Go 重走·回归检查 vs 台账33。',
  whenToUse: 'merge origin/main 后·代码层已验绿(cargo 64 suite/HX-C001)·live 环境 UP(hello洪泛)·跑全量行为 e2e 防 merge 回归',
  phases: [
    { title: '核心链重验', detail: 'FULL UC + session 补证·确认 merge 后核心 UC 行为不回归' },
    { title: 'sync+群管+广播重验', detail: 'session 现构 sync/admin + dual-account(3.x/6.1/6.4)' },
    { title: '3重跑+书签待办', detail: 'uc15-revoke(1.5) + 2.3 postId定位 + 4.3 too_long注入 + bookmark(9)+todo(10)' },
    { title: '三表对账+回归报告', detail: 'fullmap-coverage + 逐UC对比台账33·标 merge 是否引入回归 + 3重跑结果' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const BRANCH = 'main'
const LEDGER = 'docs/review/uc-coverage-ledger.md'
const E2E_ENV = `HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON='["678","555"]'`

const DOCTRINE = `
## 🔴 验证口径(已确立·橡皮章零容忍)
- 在 merge 后 main(e869a0d) 上跑·codex 已把 module.rs 拆 lifecycle/query_dispatch/send_flow/sync_flow/port_reply·须确认行为不回归。
- 发送者会收 action=post 自echo(self-omit误诊已纠)。读族(bookmark/announcement/replies/load_older)走 read-relay→im:read:result emit bytes>0。
- 每 UC 判 verdict 必须贴真 RAW WS 帧 或 im:read:result emit bytes>0 或 HTTP200;贴不出如实降级。
- 跑前 git rev-parse HEAD 确认 e869a0d(或其后)·用隔离 HELIX_DB_PATH=mktemp 防 cursor 串。
- **回归检查**:每 UC 对比台账既有状态——若台账 ✅ 但本轮 e2e 挂 = merge 引入回归(高优报)!若台账非✅ 本轮过 = 夺回。
- 不改台账(P4 收口统一)·不 commit 业务码。账号 ${E2E_ENV}。
`
const E2E_SCHEMA = {
  type: 'object',
  required: ['cluster', 'results'],
  properties: {
    cluster: { type: 'string' },
    results: {
      type: 'array',
      items: {
        type: 'object',
        required: ['uc', 'verdict', 'evidence'],
        properties: {
          uc: { type: 'string' },
          ledger_was: { type: 'string', description: '台账原状态(✅/⏭/⬜)·用于回归判定' },
          verdict: { type: 'string', enum: ['e2e-verified', 'data-dep', 'harness-gap', 'untested', 'REGRESSION'] },
          evidence: { type: 'string' },
        },
      },
    },
  },
}

// ════ Phase 1:核心链重验 ════
phase('核心链重验')
const coreR = await agent(
  `# Round-7 P1 核心链重验(merge 后回归检查)\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `跑 ① bash scripts/host-cli-FULL-uc-e2e.sh(全量 UC) ` +
  `② 用 scripts/host-cli-session.sh 现构 UC-2.2/2.4 补证序列。\n` +
  `覆盖 UC:1.1/1.5链内/2.2/2.4/5.1/5.2/5.3/6.3/1.10。每个用隔离 HELIX_DB_PATH=$(mktemp)。\n${DOCTRINE}\n` +
  `逐 UC verdict + ledger_was(查 ${LEDGER} 原状态)。**台账✅但本轮挂→标 REGRESSION 高优**。`,
  { label: 'reverify:core-chain', phase: '核心链重验', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
).catch(() => null)
log(`P1 核心链: ${coreR ? (coreR.results || []).filter((r) => r.verdict === 'REGRESSION').length : '?'} 回归`)

// ════ Phase 2:sync+群管+广播重验 ════
phase('sync+群管+广播重验')
const scenR = await agent(
  `# Round-7 P2 sync/群管/广播重验\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `跑 ① 用 scripts/host-cli-session.sh 现构 sync-gap(UC-4.3/4.4/4.5) 与 channel-admin(UC-5.4/5.5/5.6/6.2) ` +
  ` ③ bash scripts/host-cli-dual-account-e2e.sh(UC-3.1/3.4/6.1/6.4 广播·双账号)。各隔离 DB。\n${DOCTRINE}\n` +
  `逐 UC verdict + ledger_was。台账✅本轮挂→REGRESSION。`,
  { label: 'reverify:sync-admin-broadcast', phase: 'sync+群管+广播重验', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
).catch(() => null)
log(`P2 sync/群管/广播: ${scenR ? (scenR.results || []).filter((r) => r.verdict === 'REGRESSION').length : '?'} 回归`)

// ════ Phase 3:3 重跑 + 书签待办 ════
phase('3重跑+书签待办')
const reclaimR = await agent(
  `# Round-7 P3 三重跑(4.3/2.3/1.5) + 书签待办重验\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## 用户点名重跑 3 个(之前卡环境/harness):\n` +
  `- **1.5 撤回**:用 scripts/host-cli-session.sh 现构 send→拿postId→revoke→断言 post_update 带该 postId。send 回声现在通→大概率夺回。\n` +
  `- **2.3 postId 定位**:持久 host-cli 建史几条→cmd im_load_older_context {anchor_post_id:<某条真postId>} 或 im_query_messages_by_channel→看 im:messages:older_loaded/查询投影 bytes>0 含定位锚。accepts 已解封(from_tick is_query)。\n` +
  `- **4.3 too_long**:用 scripts/host-cli-session.sh 现构 too_long 段;若仍 harness-gap(无 cursor 注入入口)→如实标 harness-gap 不假判(注入钩子是另一任务)。\n` +
  `## 顺带重验:bash scripts/(书签 UC-9 / 待办 UC-10 相关·若有独立脚本则跑·否则持久链触发 queryTodoList 看 im:todo:updated)。\n${DOCTRINE}\n` +
  `逐 UC verdict + ledger_was。1.5/2.3 此轮夺回则 e2e-verified·仍卡如实标。`,
  { label: 'reverify:3reruns+bookmark-todo', phase: '3重跑+书签待办', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
).catch(() => null)
log(`P3 重跑: ${reclaimR ? (reclaimR.results || []).filter((r) => r.verdict === 'e2e-verified').length : '?'} verified`)

// ════ Phase 3.5:补漏全覆盖(无专用脚本的 12 UC·现构 host-cli 序列) ════
phase('补漏全覆盖')
const gapR = await agent(
  `# Round-7 P3.5 补漏：现成脚本够不到的 12 UC（早轮 ad-hoc 验过·无保存脚本）\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `**真·全量**要求这 12 个也在 merge 后 main 重验。用持久 host-cli（参考 scripts/host-cli-session.sh 的 FIFO 持连+wait_for）现构序列，隔离 HELIX_DB_PATH=$(mktemp)：\n` +
  `- **1.2 DOCUMENT**：自建群→cmd im_send_message {type:"DOCUMENT",...}→看 post 回声 type=DOCUMENT 原样。\n` +
  `- **1.4 重发**：send 一条→看 post 回声（重发=重走 posts/create·文本锚命中即 verified）。\n` +
  `- **1.7 转发**：im_relay_messages/createPosts {posts,channelIds}→HTTP200 SUCCESS（self 即时窗无独立帧·HTTP200 为硬证）。\n` +
  `- **1.8 emoji 表态**：im_quick_reply→posts/quickReply→看 post_update 带 quickReply[{emoji,userIds}]。\n` +
  `- **1.9 加急**：im_urgent_post→posts/urgentPost→看 post_update 带 expediteMap.recipients。\n` +
  `- **2.1 切群首屏**：对某 channel 触发 increment（hello 或 incrementByChannelId）→看 increment_channel data（=4.1 同机制·verified-by-proxy 可接受但要贴本轮帧）。\n` +
  `- **3.2 单条已读**：send→拿 postId→cmd post/read {postId}→看 post_read data.postId 命中。\n` +
  `- **3.3 模板已收到**：对 TEMPLATE 帖 cmd im_template_received {postId}→看 post_update updateAt bump + props.template.userIds 含 444。\n` +
  `- **4.1 hello 增量**：hello 握手→看 increment_channel data 真推（probe 已见·本轮再锚）。\n` +
  `- **4.2 needSync**：看 increment_channel.needSync=true 真帧。\n` +
  `- **5.7 在线**：hello 后看 status_change data.status=online user_id=444 帧（probe 已见）。\n` +
  `- **10 待办**：hello 收尾 increment_channel_end 触发 queryTodoList→看 im:todo:updated bytes>0（非空信封）。\n` +
  `${DOCTRINE}\n` +
  `逐 UC verdict + ledger_was。台账✅本轮挂→REGRESSION 高优。贴不出真证据如实降级，禁橡皮章。`,
  { label: 'reverify:gap-12', phase: '补漏全覆盖', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
).catch(() => null)
log(`P3.5 补漏12: ${gapR ? (gapR.results || []).filter((r) => r.verdict === 'REGRESSION').length : '?'} 回归`)

// ════ Phase 4:三表对账 + 回归报告 ════
phase('三表对账+回归报告')
const all = [coreR, scenR, reclaimR, gapR].filter(Boolean).flatMap((x) => x.results || [])
const collect = await agent(
  `# Round-7 收口:三表对账 + merge 回归报告\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## 全 e2e 裁决(${all.length}条):\n${JSON.stringify(all, null, 1).slice(0, 9000)}\n\n` +
  `任务:\n` +
  `1. 跑 bash scripts/fullmap-coverage.sh 记三表(HTTP/WS/UC) — 确认 merge 后仍满/不回退。\n` +
  `2. **回归报告**(最高优):列出所有 verdict=REGRESSION(台账✅但本轮 e2e 挂)的 UC——这是 merge 引入的破坏,必须高亮。若零 REGRESSION→声明 merge 行为零回归。\n` +
  `3. 按本轮裁决更新 ${LEDGER}:夺回的(1.5/2.3 若过)→✅;REGRESSION→降级+标 merge 回归原因;其余维持。\n` +
  `4. SESSION.md §0 终态行:✅/⚠️ 标明 merge 后三表数 + 零回归/N回归 + 3重跑结果(1.5/2.3/4.3)。\n` +
  `5. cargo test -p helix-im 终验 + structure-gate exit 0 + git diff -- crates/helix-core/ 空。commit(docs scope·分支 ${BRANCH})。不 push(留主对话)。\n` +
  `返回 {三表数, REGRESSION清单(空=零回归), 3重跑结果, 最终UC覆盖, SESSION行}。`,
  { label: 'collect:round7', phase: '三表对账+回归报告', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
).catch(() => null)

return {
  branch: BRANCH,
  regressions: all.filter((r) => r.verdict === 'REGRESSION').map((r) => r.uc),
  reruns_3: all.filter((r) => ['UC-1.5', 'UC-2.3', 'UC-4.3'].some((u) => (r.uc || '').includes(u.replace('UC-', '')))).map((r) => ({ uc: r.uc, verdict: r.verdict })),
  collect,
}
