export const meta = {
  name: 'cses-round5-uc-reachable',
  description: 'Round-5 定向冲可达 UC:1.6编辑真相(读现网wire)+sync-gap剧本(4.3/4.4/4.5)+书签9/待办10/上拉2.2/模板3.3 用持久harness真Go e2e。明确跳过1.3(物理够不到)。',
  whenToUse: '续 Round-4(UC 22/37);分支 feat/cses-round3-conformance-e2e;持久harness已建;跳过1.3文件上传;诚实记物理/数据够不到的',
  phases: [
    { title: '1.6编辑真相', detail: '读现网cses-client编辑wire→定correct端点/body→helix对齐修→e2e验回声' },
    { title: 'sync-gap剧本', detail: '构造cursor gap态剧本:4.3 too_long大gap/4.4心跳piggyback/4.5陌生channel' },
    { title: 'e2e可达场景', detail: '持久harness跑书签9/待办10/上拉2.2/模板3.3/sync-gap,逐UC真RAW帧裁决' },
    { title: '收口对账', detail: '翻台账(叠加22不回退)+coverage+SESSION,诚实标物理够不到' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const CSES = '/Users/mac28/workspace/angular/cses-client'
const BRANCH = 'feat/cses-round3-conformance-e2e'
const PARTIALS = 'crates/helix-im/docs/csesapi-migration/full-map/partials'
const CURL = 'crates/helix-im/docs/csesapi-migration/真机curl真源.md'
const LEDGER = 'docs/review/uc-coverage-ledger.md'
const PERSIST = 'scripts/host-cli-go-chain-e2e-persistent.sh'
const E2E_ENV = `HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON='["678","555"]'`

const NAMING = `## 命名陷阱:camelCase·createAt(非createdAt)·unReadPostId(大写R)·messageType值 SYSTEN·temporaryId。`
const CARGO = `## Cargo纪律:中途禁cargo;commit前 cargo check -p helix-im + cargo test -p helix-im 各一次(≤2-3次)。守HX-C001:git diff -- crates/helix-core/ 必须空。`
const DOCTRINE = `
## 🔴 e2e 验证口径
- 发送者会收 action=post 自echo(self-omit误诊已纠)。持久连接(${PERSIST})整条链1次hello,wait_for回声即返,迟到帧落同一LOG。
- 判 verified 必须贴真 RAW WS 帧 或 HTTP200+落库证据;贴不出如实降级(data-dep/untested/harness-gap)。橡皮章零容忍(Round-3/4 已有 agent 过度声称被 collector 砍的教训)。
- **1.3 文件上传明确跳过**(host-cli 无 processMessageFiles 管线·物理够不到),不在本轮范围。
- 物理/数据够不到 → harness-gap/data-dep,**禁假判 verified**。
`
const E2E_SCHEMA = {
  type: 'object',
  required: ['scenario', 'results'],
  properties: {
    scenario: { type: 'string' },
    results: {
      type: 'array',
      items: {
        type: 'object',
        required: ['uc', 'verdict', 'evidence'],
        properties: {
          uc: { type: 'string' },
          action_expected: { type: 'string' },
          echo_seen: { type: 'boolean' },
          verdict: { type: 'string', enum: ['e2e-verified', 'data-dep', 'harness-gap', 'untested', 'wire-bug'] },
          evidence: { type: 'string', description: 'RAW WS帧/HTTP code/阻断原因' },
        },
      },
    },
  },
}

// ════ Phase 1:UC-1.6 编辑真相(读现网wire→对齐修→e2e)════
phase('1.6编辑真相')
const editFix = await agent(
  `# Round-5 P1:UC-1.6 编辑消息真相调查 + 修复\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## 背景:Round-4 P1 假设「现网编辑=posts/create 带非空 id,Go 当 update」→ 真 Go **零回声**,该假设**未经真抓包·疑似错**。\n` +
  `## 调查(行为真源=现网 cses-client,CLAUDE.md §4.5):\n` +
  `1. grep ${CSES}/src(主checkout 现网 legacy 非helix路径)message 编辑相关:editMessage/updateMessage/updatePost/编辑。找**真实端点+body**(可能是 posts/update / posts/editPost / 别的)。\n` +
  `2. 对照 ${PARTIALS}/1--backend-http-posts.md 全 24 端点 + ${CURL},确认编辑走哪条 wire。\n` +
  `3. 若 helix 缺该 outbound 命令 → **真实现**(distributed_slice 注册 + build 返 endpoint+body,对齐真源体);若 send_build id 透传方向对但端点错 → 改端点。\n` +
  `${NAMING}\n${CARGO}\n\n` +
  `4. 改完起持久 host-cli 真 Go e2e:发一条消息→编辑它→看真回声 action(post_update? 还是别的)。${E2E_ENV}。\n` +
  `TDD。返回 {真编辑端点/body, helix是否缺命令, commit SHA, e2e真回声action, UC-1.6 verdict(verified/wire-bug/data-dep) + RAW帧证据}。不push。`,
  { label: 'fix:edit-1.6', phase: '1.6编辑真相', agentType: 'cses-client-dev', effort: 'high' }
)
log(`P1 UC-1.6 编辑真相调查完成`)

// ════ Phase 2:sync-gap 剧本(构造 cursor gap 态)════
phase('sync-gap剧本')
const gapScript = await agent(
  `# Round-5 P2:sync-gap e2e 剧本(4.3 too_long / 4.4 心跳 / 4.5 陌生channel)\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `这三个 UC 代码已实现(Round-3 bf25307:4.4 pong消费+4.5陌生channel不丢帧;4.3 emit_sync_too_long),缺的是**构造触发态的 e2e 剧本**。\n` +
  `新建 scripts/host-cli-sync-gap-e2e.sh(参考 ${PERSIST} 的持久连接+FIFO+wait_for):\n` +
  `- **4.5 陌生 channel**:对一个**本地cursor store没有**的 channelId 触发 im_channel_load_increment_by_channel_id → 看 increment_channel 帧不丢(自动注册)。\n` +
  `- **4.4 心跳 gap**:持久连接挂住 → 等 ping 周期(或手动触发)→ 看 ping 帧体带 cursors/allHash piggyback(sync_increment.rs)。\n` +
  `- **4.3 too_long**:构造大 cursor gap(本地 cursor 远落后服务端 seq)→ hello/increment 触发 → 看 too_long 帧 + 重拉。若 host-cli 无法构造足够大 gap → 诚实记 harness-gap(不假判)。\n` +
  `${DOCTRINE}${E2E_ENV}\n` +
  `冒烟跑通的标 e2e-verified,构造不出态的标 harness-gap/data-dep。返回 {脚本路径, 每UC verdict+RAW帧证据, commit SHA(scripts scope)}。不push。`,
  { label: 'script:sync-gap', phase: 'sync-gap剧本', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
)
log(`P2 sync-gap 剧本完成`)

// ════ Phase 3:e2e 可达场景(持久harness·串行)════
phase('e2e可达场景')
const SCEN = [
  { key: 'bookmark-9', ucs: 'UC-9.1 书签创建·9.2 加载·9.3 删除',
    how: 'im_bookmark_create/load/delete(已wired)。持久连接跑三命令,看 HTTP200 + im:read:result/对应回声。Round-4 曾报verified但证据不足被降级→本轮给干净RAW/HTTP证据。' },
  { key: 'todo-10', ucs: 'UC-10 通知/待办',
    how: 'todo装配链:先在群里制造 mention/urgent 消息(send 带 mentions)→ 触发 hello收尾 queryTodoList → 看 im:todo:updated 投影帧(含装配的 items)。需 mention 数据态。' },
  { key: 'pull-2.2', ucs: 'UC-2.2 上拉历史',
    how: 'im_load_older_context:在有历史的 channel(先 send 几条)上触发上拉翻页 → 看 im:messages:older_loaded 投影(messages[]/hasMore)。' },
  { key: 'template-3.3', ucs: 'UC-3.3 模板已收到',
    how: 'post/templateReceived(Round-3 c7a840c 修过 camel 键)。需模板消息态;构造不出则 data-dep。看 post_update 回声。' },
]
const e2e = []
for (const s of SCEN) {
  const r = await agent(
    `# Round-5 e2e 场景 ${s.key}\ncwd ${HELIX}(分支 ${BRANCH})。真后端已起。持久harness ${PERSIST} 可参考/复用。\n` +
    `## UC:${s.ucs}\n## 跑法:${s.how}\n${DOCTRINE}${E2E_ENV}\n` +
    `逐UC verdict + RAW帧/HTTP证据。构造不出数据态→data-dep,命令未达→untested,贴不出证据→不准verified。`,
    { label: `e2e:${s.key}`, phase: 'e2e可达场景', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
  ).catch(() => null)
  e2e.push({ s: s.key, r })
  const v = r ? (r.results || []).filter((x) => x.verdict === 'e2e-verified').length : 0
  log(`e2e场景 ${s.key}: ${v}/${r ? (r.results || []).length : 0} verified`)
}

// ════ Phase 4:收口对账 ════
phase('收口对账')
const verdicts = [
  ...(gapScript && gapScript.results ? gapScript.results : []),
  ...e2e.filter((x) => x && x.r).flatMap((x) => x.r.results || []),
]
const collect = await agent(
  `# Round-5 收口:翻台账+三表对账+SESSION\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## P1 UC-1.6 编辑调查结论:${typeof editFix === 'string' ? editFix.slice(0, 1500) : JSON.stringify(editFix).slice(0, 1500)}\n\n` +
  `## P2/P3 e2e裁决(${verdicts.length}条):\n${JSON.stringify(verdicts, null, 1).slice(0, 8000)}\n\n` +
  `任务:\n` +
  `1. Edit ${LEDGER}:按裁决更新(e2e-verified→✅;data-dep→⏭;harness-gap→⬜注明物理够不到;wire-bug→🔴)。**Round-4 已verified的22不回退**。1.3 文件上传保持⬜并注明「host-cli物理够不到·需真前端」。\n` +
  `2. 跑 bash scripts/fullmap-coverage.sh 记三表 + UC新数。\n` +
  `3. SESSION.md §0 终态行:✅/⚠️ PARTIAL(列剩余UC+诚实物理/数据阻因)。\n` +
  `4. cargo test -p helix-im + git diff -- crates/helix-core/ 空(HX-C001)。commit(docs+scripts scope)。不push。\n` +
  `返回 {三表数, 台账三态分布, Round-5净增verified的UC, 剩余UC及物理/数据阻因, SESSION行}。`,
  { label: 'collect:round5', phase: '收口对账', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
)

return {
  branch: BRANCH,
  edit_1_6: typeof editFix === 'string' ? editFix.slice(0, 300) : editFix,
  e2e_verified: verdicts.filter((v) => v.verdict === 'e2e-verified').map((v) => v.uc),
  remaining: verdicts.filter((v) => v.verdict !== 'e2e-verified').map((v) => ({ uc: v.uc, verdict: v.verdict })),
  collect,
}
