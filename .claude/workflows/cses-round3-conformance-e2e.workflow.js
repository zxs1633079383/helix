export const meta = {
  name: 'cses-round3-conformance-e2e',
  description: 'Round-3:逐UC契约审计(对抗式·非橡皮章) → 真偏差修复(helix新分支) → 27UC真Go e2e串行翻台账 → 三表对账。修正 self-omit 误诊(发送者会收 post 回声)。',
  whenToUse: '续 SESSION §0 HANDOFF 两任务;分支 feat/cses-round3-conformance-e2e;环境tp已通WS hello真洪泛;排除 search集合七 + createUser/status(goBlocked)',
  phases: [
    { title: '契约审计', detail: '7域并行对抗式审计:逐UC对齐full-map/partials+真机curl真源,挑真偏差不准盖章' },
    { title: '偏差修复', detail: 'helix-im outbound/投影 串行修真偏差(共享god-file→串行)+cargo test;无偏差则诚实no-op' },
    { title: 'e2e串行验证', detail: '6场景对真Go跑,修正口径(发送者post回声+广播类双账号),逐UC出裁决' },
    { title: '三表对账', detail: '收口写台账+fullmap-coverage.sh+SESSION DONE' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const CW = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ'
const BRANCH = 'feat/cses-round3-conformance-e2e'
const PARTIALS = 'crates/helix-im/docs/csesapi-migration/full-map/partials'
const CURL = 'crates/helix-im/docs/csesapi-migration/真机curl真源.md'
const LEDGER = 'docs/review/uc-coverage-ledger.md'
const E2E_ENV = `HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON='["678","555"]' SETTLE_MS=2800`

// ─── 注入常量 ───────────────────────────────────────────────
const NAMING = `## 命名陷阱(逐字核,错一个=偏差):camelCase(非snake)·createAt(非createdAt,B3)·unReadPostId(大写R)·messageType值 SYSTEN(非SYSTEM)·temporaryId·channelId/teamId/userId·posts/create(单发)vs /posts/createPosts(转发批量,双段复数)。`

const FULLCODE = `## 🔴 反橡皮章铁律
- 你是**对抗式审计员**,默认假设"代码可能不符合UC",去**证伪**。不准草草"已符合"交差。
- 判"符合"必须贴**三件证据**:① full-map规格端点 ② helix实际build()返回端点/body ③ 逐字diff结论。
- 判"偏差"必须给:UC编号·偏差类型(端点/字段集/命名/缺命令)·full-map原文锚·helix现状锚·修复指向。
- **禁**"前端零caller故跳过":除非契约审计证明现状已符合,否则记为真偏差。
- 唯一可不审:search(集合七)+createUser/status(Go物理缺handler)。`

const CARGO = `## Cargo纪律:中途禁cargo;改完一模块commit前 cargo check -p helix-im + cargo test -p helix-im 各一次(≤2-3次)。守HX-C001:git diff -- crates/helix-core/ 必须空(不准动core)。`

const AUDIT_SCHEMA = {
  type: 'object',
  required: ['domain', 'ucs_audited', 'findings', 'clean'],
  properties: {
    domain: { type: 'string' },
    ucs_audited: { type: 'array', items: { type: 'string' } },
    clean: { type: 'boolean', description: '该域逐UC全符合UC契约、零真偏差' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        required: ['uc', 'kind', 'fullmap_anchor', 'helix_anchor', 'fix_hint', 'severity'],
        properties: {
          uc: { type: 'string' },
          kind: { type: 'string', enum: ['endpoint', 'body_fields', 'naming', 'missing_command', 'cses_wiring', 'other'] },
          fullmap_anchor: { type: 'string', description: 'full-map/partial 文件:行 或 真源原文' },
          helix_anchor: { type: 'string', description: 'helix 文件:行 现状' },
          fix_hint: { type: 'string' },
          severity: { type: 'string', enum: ['blocker', 'major', 'minor'] },
        },
      },
    },
  },
}

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
          action_expected: { type: 'string', description: '期待的WS action回声(如 post/channel_created)' },
          echo_seen: { type: 'boolean' },
          verdict: { type: 'string', enum: ['e2e-verified', 'data-dep', 'go-blocked', 'wire-bug', 'untested'] },
          evidence: { type: 'string', description: 'RAW WS帧片段 或 HTTP code 或 阻断原因' },
        },
      },
    },
  },
}

// ════════════════════ Phase 1:契约审计(7域并行·对抗式) ════════════════════
phase('契约审计')
const DOMAINS = [
  { key: 'posts', ucs: 'UC-1.1~1.10(发送/撤回/编辑/转发/emoji/加急/定时)+UC-2.1~2.4(切群首屏/上拉/定位/回复列表)',
    refs: `${PARTIALS}/1--backend-http-posts.md + ${PARTIALS}/9--gap-posts.md`,
    helix: 'crates/helix-im/src/outbound/send_build.rs + posts_existing.rs + channel_write/post.rs + older_context.rs' },
  { key: 'read', ucs: 'UC-3.1~3.4(会话已读/单条已读/模板已收到/未读数)',
    refs: `${PARTIALS}/1--backend-http-posts.md(read段) + ${PARTIALS}/9`,
    helix: 'crates/helix-im/src/outbound/posts_read.rs + read_relay.rs + module.rs(OutboundReadReply)' },
  { key: 'sync', ucs: 'UC-4.1~4.5(hello增量/needSync/too_long重拉/心跳gap/陌生channel)',
    refs: `${PARTIALS}/2--backend-http-channel-sync.md + ${PARTIALS}/5--backend-ws-events.md + ${PARTIALS}/10`,
    helix: 'crates/helix-im/src/sync_increment.rs + channel_read.rs + ws/handlers/increment_channel*.rs + ws/handlers/hello.rs' },
  { key: 'channel', ucs: 'UC-5.1~5.7(建群/话题/关退群/群属性/置顶/公告/在线分组)',
    refs: `${PARTIALS}/2--backend-http-channel-sync.md + ${PARTIALS}/10--gap-channel-sync.md`,
    helix: 'crates/helix-im/src/channel.rs + channel_write/*.rs + outbound/registry.rs' },
  { key: 'member', ucs: 'UC-6.1~6.4(拉踢/管理员/群昵称/成员快照)',
    refs: `${PARTIALS}/2 + ${PARTIALS}/10`,
    helix: 'crates/helix-im/src/channel_write/*.rs(member/role/nickname) + outbound/' },
  { key: 'vote-bookmark', ucs: 'UC-8(投票/平均分)+UC-9(书签)+UC-5.7在线兜底',
    refs: `${PARTIALS}/3--backend-http-user-misc.md + ${PARTIALS}/11--gap-user-misc.md`,
    helix: 'crates/helix-im/src/outbound/vote_score.rs + user_misc 写族' },
  { key: 'todo-bot', ucs: 'UC-10(通知/待办装配链)+bot-agent域(多数前端0caller→须契约审计证明)',
    refs: `${PARTIALS}/3 + ${PARTIALS}/4--backend-http-bot-agent.md + ${PARTIALS}/12--gap-bot-agent.md`,
    helix: 'crates/helix-im/src/todo.rs + outbound/(bot_agent)' },
]

const audits = await parallel(DOMAINS.map((d) => () =>
  agent(
    `# Round-3 契约审计·对抗式 —— 域 ${d.key}\ncwd ${HELIX}(分支 ${BRANCH})。**只读审计,不改代码不commit**。\n` +
    `## 审计范围 UC:${d.ucs}\n` +
    `## full-map 硬契约真源(逐字对齐):${d.refs}\n## body真抓包真源:${CURL}\n## helix 现状落点:${d.helix}\n` +
    `## 另审 cses 下放符合性(若该域涉前端渲染):${CW}(分支 feat/helix-takeover-integ) 的 8cba55e4..HEAD 下放 commit —— helix invoke/投影是否真接、渲染路径是否完整。\n\n` +
    `${FULLCODE}\n${NAMING}\n\n` +
    `逐 UC 走:① 端点对不对(vs full-map真路径,注意 posts/create vs createPosts)② body字段集对不对(vs ${CURL}逐字)③ 命名陷阱 ④ 命令是否真注册(distributed_slice OUTBOUND_COMMANDS,grep registry)⑤ cses下放是否真接线(非结论)。\n` +
    `返回结构化 findings(每条真偏差带 full-map锚+helix锚+修复指向+severity);该域逐UC全符合则 clean=true 且 findings=[](但 clean=true 必须是你贴了三件证据证伪未果的结论,不是默认)。`,
    { label: `audit:${d.key}`, phase: '契约审计', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: AUDIT_SCHEMA }
  ).then((r) => ({ d: d.key, r })).catch(() => ({ d: d.key, r: null }))
))

const allFindings = audits.filter((x) => x && x.r).flatMap((x) => (x.r.findings || []).map((f) => ({ ...f, domain: x.d })))
const realDeviations = allFindings.filter((f) => f.severity === 'blocker' || f.severity === 'major')
log(`审计完成:${audits.length}域,真偏差(major+) ${realDeviations.length} 条,minor ${allFindings.length - realDeviations.length} 条`)

// ════════════════════ Phase 2:偏差修复(串行·共享god-file) ════════════════════
phase('偏差修复')
// 按域分组真偏差;有偏差的域串行修(module.rs/to_effect.rs 共享→禁并行commit,HX-C013)
const devByDomain = {}
for (const f of realDeviations) (devByDomain[f.domain] ||= []).push(f)
const fixResults = []
if (Object.keys(devByDomain).length === 0) {
  log('🟢 P1 审计零真偏差(major+) → P2 诚实 no-op,不造假改动(守 HX-C009/C012)。直接进 e2e。')
} else {
  for (const [dom, devs] of Object.entries(devByDomain)) {
    const r = await agent(
      `# Round-3 偏差修复 —— 域 ${dom}\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
      `## 待修真偏差(${devs.length}条,来自对抗式审计):\n` +
      devs.map((f, i) => `${i + 1}. [${f.uc}/${f.kind}/${f.severity}] full-map:${f.fullmap_anchor} | helix:${f.helix_anchor} | 修复:${f.fix_hint}`).join('\n') +
      `\n\n${FULLCODE}\n${NAMING}\n${CARGO}\n\n` +
      `逐条修真代码(outbound端点/body/命名/补命令);TDD 先写测试性质再实现。**禁动 helix-core**。改完 cargo check -p helix-im + cargo test -p helix-im 绿才 commit(结构化body)。` +
      `返回 {commit SHA, 改的文件, cargo结果, 每条偏差是否已修(真修/无法修原因)}。不push。`,
      { label: `fix:${dom}`, phase: '偏差修复', agentType: 'rust-dev', effort: 'high' }
    )
    fixResults.push({ dom, r })
    log(`fix域 ${dom}: ${devs.length}条偏差处理完`)
  }
}

// ════════════════════ Phase 3:e2e 串行验证(6场景·修正口径) ════════════════════
phase('e2e串行验证')
const E2E_DOCTRINE = `
## 🔴 验证口径(修正 self-omit 误诊,真源 ${CURL}:5 背书)
- **self-omit 是误诊**:发送者**会**收到自己消息的 WS 回声,action=**post**(非posted)。send无回声的真因是**body不对**,已修。
- **send族验证**(UC-1.x):发送者自己连接看到 action=post 回声 = verified。
- **广播族验证**(UC-6.1拉踢/6.2管理员/6.4快照等"广播给被操作方非self"):用双账号——主连接(444)操作,**第二连接(678)** 收 update/member 广播 = verified。需起两个 host-cli 连接。
- **data-dep族**(已读UC-3.x需未读态):双账号造未读态(678发→444读→看 post_read/未读数回声)。
- 跑法:host-cli 逐行喂命令(参考 scripts/host-cli-go-chain-e2e.sh),HELIX_DUMP_FRAMES=1 抓 RAW WS inbound 帧,grep "action":"<期待>"。账号 ${E2E_ENV}。
- **不改台账**(P4收口统一写)。不commit。每UC给 verdict+RAW帧证据。
- 遇 gRPC unknown/无hello → 报环境阻断(不假判 wire-bug)。
`
const SCENARIOS = [
  { key: 'core-chain', ucs: 'UC-1.1发送·UC-1.5撤回·UC-2.2上拉·UC-5.1建群·UC-5.2话题·UC-5.3关退群·UC-6.3群昵称·UC-1.10定时',
    how: '直接跑 bash scripts/host-cli-go-chain-e2e.sh(13动作自建群链),解析RAW帧逐动作判 verdict。' },
  { key: 'send-family', ucs: 'UC-1.2 DOCUMENT·UC-1.3图片文件·UC-1.4重发·UC-1.6编辑·UC-1.7转发·UC-1.8 emoji·UC-1.9加急',
    how: '自建群后,逐类型 send(走posts/create或对应端点),看发送者 post 回声;编辑走 updateMessage 看回声;加急看 urgent 字段帧。' },
  { key: 'read-unread', ucs: 'UC-3.1会话已读·UC-3.2单条已读·UC-3.3模板已收到·UC-3.4未读数',
    how: '双账号:678 向自建群发→444 读(channel/read)→看 post_read/未读数回声;模板已收到走 received 端点。' },
  { key: 'channel-admin', ucs: 'UC-5.4群属性·UC-5.5消息置顶·UC-5.6群公告·UC-6.2管理员·UC-6.4成员快照',
    how: '自建群后跑 channel/change/info·post_pin·announcement·member_role 双账号广播·成员查询,看各自回声。' },
  { key: 'sync', ucs: 'UC-4.1 hello增量·UC-4.2 needSync·UC-4.3 too_long重拉·UC-4.4心跳gap·UC-4.5陌生channel',
    how: 'hello握手看 increment_channel data真推+needSync帧;too_long需造大gap剧本;心跳ping看piggyback;陌生channel走 incrementByChannelId。' },
  { key: 'vote-todo-online', ucs: 'UC-8投票平均分·UC-9书签·UC-5.7在线分组·UC-10通知待办',
    how: '自建群跑 vote/vote+average/*看回声;书签端点;在线看 status_change online帧(probe已见);待办 hello收尾 queryTodoList 看 im:todo:updated。' },
]
const e2eResults = []
for (const s of SCENARIOS) {
  const r = await agent(
    `# Round-3 e2e 场景 ${s.key}\ncwd ${HELIX}(分支 ${BRANCH})。真后端栈已起(tp通·WS hello真洪泛已验)。\n` +
    `## 本场景 UC:${s.ucs}\n## 跑法:${s.how}\n${E2E_DOCTRINE}\n` +
    `逐 UC 出 verdict:e2e-verified(看到期待回声)/data-dep(命令达Go但回声需特定数据态且本轮未造出)/go-blocked(Go物理缺)/wire-bug(体/端点错致Go不推,记细节)/untested。带 RAW WS 帧证据。`,
    { label: `e2e:${s.key}`, phase: 'e2e串行验证', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
  ).catch(() => null)
  e2eResults.push({ s: s.key, r })
  const v = r ? (r.results || []).filter((x) => x.verdict === 'e2e-verified').length : 0
  log(`e2e场景 ${s.key}: ${v}/${r ? (r.results || []).length : 0} verified`)
}

// ════════════════════ Phase 4:三表对账(收口写台账) ════════════════════
phase('三表对账')
const verdicts = e2eResults.filter((x) => x && x.r).flatMap((x) => x.r.results || [])
const collector = await agent(
  `# Round-3 收口:写台账 + 三表对账 + SESSION DONE\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## e2e 裁决(${verdicts.length}条,来自6场景):\n` +
  JSON.stringify(verdicts, null, 1).slice(0, 8000) + '\n\n' +
  `## P1审计真偏差修复情况:\n` + JSON.stringify(fixResults.map((x) => ({ dom: x.dom })), null, 1) + '\n\n' +
  `任务:\n` +
  `1. Edit ${LEDGER}:按 e2e 裁决更新每 UC 状态(e2e-verified→✅ e2e-verified;data-dep→⏭;wire-bug→🔴记原因)。**修正 self-omit 误标**:台账头部"self-omit已证"段改为"误诊已纠:发送者会收post回声,见真机curl真源:5",send族按真裁决标。\n` +
  `2. 跑 bash scripts/fullmap-coverage.sh,记三表(HTTP/WS/UC)覆盖数。in-scope 100% 判据(排除search+createUser/status)。\n` +
  `3. 写 SESSION.md §0 终态行:✅ DONE round-3 契约审计+e2e 或 ⚠️ PARTIAL(列剩余 data-dep/wire-bug)。含 commit 范围、三表数、台账三态分布。\n` +
  `4. cargo test -p helix-im 终验绿 + git diff -- crates/helix-core/ 空(HX-C001)。commit 台账+SESSION(docs scope)。不push。\n` +
  `返回 {三表覆盖数, 台账三态分布, SESSION DONE行, 剩余未达100%的UC及原因}。`,
  { label: 'collect:ledger+对账', phase: '三表对账', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
)

return {
  branch: BRANCH,
  audit_domains: audits.length,
  real_deviations: realDeviations.length,
  fixed_domains: fixResults.map((x) => x.dom),
  e2e_verified: verdicts.filter((v) => v.verdict === 'e2e-verified').map((v) => v.uc),
  remaining: verdicts.filter((v) => v.verdict !== 'e2e-verified').map((v) => ({ uc: v.uc, verdict: v.verdict })),
  collector,
}
