export const meta = {
  name: 'cses-round4-uc-coverage-push',
  description: 'Round-4 全力冲 UC 覆盖:修2真wire-bug(type/id透传) → 建双账号e2e harness → 多场景真Go e2e(双账号/gap剧本/vote第二网关:3399/channel-admin重跑) → 收口翻台账。',
  whenToUse: '续 Round-3(UC 12/37);分支 feat/cses-round3-conformance-e2e;环境已通;vote走:3399第二网关;文件上传UC-1.3物理够不到诚实记untested',
  phases: [
    { title: '修wire-bug', detail: 'send_build BuildInput+handle_send_message 加 msg_type(默认TEXT)+id(默认"")透传 → UC-1.2 DOCUMENT/1.6 编辑' },
    { title: '建双账号harness', detail: 'scripts/host-cli-dual-account-e2e.sh:444发+678收,断言广播帧到达接收方,冒烟验通' },
    { title: 'e2e多场景', detail: '双账号(已读/拉踢/快照/转发)+channel-admin重跑+vote第二网关+gap剧本+书签待办,逐UC裁决' },
    { title: '收口对账', detail: '翻台账+fullmap-coverage.sh+SESSION DONE,诚实记物理够不到的UC' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const BRANCH = 'feat/cses-round3-conformance-e2e'
const PARTIALS = 'crates/helix-im/docs/csesapi-migration/full-map/partials'
const CURL = 'crates/helix-im/docs/csesapi-migration/真机curl真源.md'
const LEDGER = 'docs/review/uc-coverage-ledger.md'
const E2E_ENV = `HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON='["678","555"]' SETTLE_MS=2800`

const NAMING = `## 命名陷阱:camelCase·createAt(非createdAt)·unReadPostId(大写R)·messageType值 SYSTEN·temporaryId·posts/create(单发,id非空=编辑update)。`
const CARGO = `## Cargo纪律:中途禁cargo;commit前 cargo check -p helix-im + cargo test -p helix-im 各一次(≤2-3次)。守HX-C001:git diff -- crates/helix-core/ 必须空。`
const DOCTRINE = `
## 🔴 e2e 验证口径(已确立)
- send无回声真因=body不对(非self-omit);发送者**会**收 action=post 自echo。send族=发送者自连接看 post 回声。
- 广播族(拉踢6.1/快照6.4/已读3.x未读态)=双账号:444操作,678第二连接收 update/member/post_read 广播。
- vote(UC-8)走**第二网关**:跑 e2e 时加 env HELIX_DEFAULT_API_BASE=http://127.0.0.1:3399(已探活);验 HTTP 200 或对应回声。
- 文件上传(UC-1.3)host-cli无 processMessageFiles 管线→**物理够不到,诚实记 untested(harness-gap),禁假判**。
- RAW WS 帧证据(HELIX_DUMP_FRAMES=1)grep action;不改台账(P4收口);不push。
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
          verdict: { type: 'string', enum: ['e2e-verified', 'data-dep', 'go-blocked', 'wire-bug', 'harness-gap', 'untested'] },
          evidence: { type: 'string', description: 'RAW WS帧片段/HTTP code/阻断原因' },
        },
      },
    },
  },
}

// ════ Phase 1:修 2 wire-bug(单agent,送入站 type/id 透传)════
phase('修wire-bug')
const fix = await agent(
  `# Round-4 P1 修两条 send wire-bug\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## 偏差(Round-3 审计实锤):\n` +
  `1. **UC-1.2 DOCUMENT**:send_build.rs:160 硬编 type=json!("TEXT") → 文档/图片消息降级纯文本。\n` +
  `2. **UC-1.6 编辑**:send_build.rs:175 硬编 id=json!("") → 编辑丢命令id,永走新建而非 update(现网编辑=posts/create 带真post id,id非空 Go当update)。\n\n` +
  `## 修法:\n` +
  `- BuildInput 加 \`msg_type: &'a str\`(默认"TEXT")+ \`post_id: &'a str\`(默认"",编辑时传真id)字段。\n` +
  `- build_message_object body 写 type=msg_type、id=post_id(替换两处硬编)。\n` +
  `- handle_send_message(module.rs:982) 解析命令载荷的 type/id(零信任 require/default),透传进 BuildInput。\n` +
  `- 对照 ${PARTIALS}/1(posts/create 行 post_update echo)+ ${CURL}§1。${NAMING}\n` +
  `${CARGO}\n\n` +
  `TDD:先写 send_build 测试(type=DOCUMENT 透传 / post_id 非空走编辑 / 默认值不回归)。改完 cargo test -p helix-im 绿才 commit(结构化body)。不push。` +
  `返回 {commit SHA, 改的文件, cargo结果, 新增测试}。`,
  { label: 'fix:send-type-id', phase: '修wire-bug', agentType: 'rust-dev', effort: 'high' }
)
log(`P1 wire-bug 修复完成`)

// ════ Phase 2:建双账号 e2e harness(单agent)════
phase('建双账号harness')
const harness = await agent(
  `# Round-4 P2 建双账号 e2e harness\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `新建 scripts/host-cli-dual-account-e2e.sh(参考现有 scripts/host-cli-go-chain-e2e.sh 结构):\n` +
  `- 起**两个并发 host-cli 连接**:接收方(678)先连、idle 持续抓 RAW 帧到 RX_LOG;发送方(444)自建群+拉678入群,在群里 send/拉踢/改成员,各动作后 settle。\n` +
  `- 断言:接收方 RX_LOG 出现广播帧(post / member_change / update_channel / post_read),发送方自身回声照常。\n` +
  `- 账号 ${E2E_ENV};接收方用 HELIX_COOKIE_ID=678。两连接独立 HELIX_DB_PATH(避免 cursor 串)。\n` +
  `- 冒烟:真连 Go 跑一遍 send,确认 678 收到 post 广播(444 建群拉678入群后发) → 证 harness work。\n` +
  `${DOCTRINE}\n` +
  `**只新建脚本不改生产代码**;脚本可执行 chmod +x;冒烟通过后 commit(scripts scope)。返回 {脚本路径, 冒烟结果(678是否收到广播), commit SHA}。`,
  { label: 'harness:dual-account', phase: '建双账号harness', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
)
log(`P2 双账号 harness:冒烟 ${typeof harness === 'string' ? harness.slice(0, 80) : ''}`)

// ════ Phase 3:e2e 多场景(串行)════
phase('e2e多场景')
const SCEN = [
  { key: 'send-types', ucs: 'UC-1.2 DOCUMENT(P1修后type透传)·UC-1.6 编辑(P1修后id透传走update)',
    how: '自建群,send type=DOCUMENT 看 post 回声 type=DOCUMENT;再带真 post_id 发(编辑)看 post_update 回声 id 一致。' },
  { key: 'dual-unread-broadcast', ucs: 'UC-3.1会话已读·UC-3.4未读数·UC-6.1拉踢·UC-6.4成员快照·UC-1.7转发',
    how: '用 P2 的 host-cli-dual-account-e2e.sh:678发→444读看post_read/未读;444拉踢678看member广播到678;成员查询;转发到目标channel看广播。' },
  { key: 'channel-admin', ucs: 'UC-5.4群属性·UC-5.5置顶·UC-5.6公告·UC-6.2管理员',
    how: '自建群单连接跑 channel/change/info·post pin·announcement save/list·member role,看各自WS回声(Round-3 e2e agent曾报verified但证据不足被降级→本轮给干净RAW帧证据或如实data-dep)。' },
  { key: 'vote-second-gw', ucs: 'UC-8 投票/平均分',
    how: '加 env HELIX_DEFAULT_API_BASE=http://127.0.0.1:3399(已探活)跑 vote/vote+average/*;验 HTTP 200 或回声。若vote结果不走:8065 WS而是HTTP同步返回,记 verified(HTTP证据)。' },
  { key: 'sync-gap', ucs: 'UC-4.3 too_long重拉·UC-4.4心跳gap·UC-4.5陌生channel',
    how: '4.3造大gap(cursor落后)看 too_long+重拉;4.4心跳ping看piggyback cursors;4.5对未知channelId incrementByChannelId看不丢帧。需构造特定sync态。' },
  { key: 'bookmark-todo', ucs: 'UC-9书签·UC-10通知待办',
    how: '书签 bookmark/save+load 看回声/HTTP;待办 hello收尾 queryTodoList 看 im:todo:updated 投影(需有 mention/urgent 数据态)。' },
]
const e2e = []
for (const s of SCEN) {
  const r = await agent(
    `# Round-4 e2e 场景 ${s.key}\ncwd ${HELIX}(分支 ${BRANCH})。真后端已起。P1 type/id已修、P2 双账号harness已建。\n` +
    `## UC:${s.ucs}\n## 跑法:${s.how}\n${DOCTRINE}\n账号 ${E2E_ENV}。\n` +
    `逐UC verdict(e2e-verified/data-dep/go-blocked/wire-bug/harness-gap/untested)+RAW帧证据。**橡皮章零容忍**:判verified必须贴真RAW WS帧或HTTP200证据,贴不出就如实降级。`,
    { label: `e2e:${s.key}`, phase: 'e2e多场景', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
  ).catch(() => null)
  e2e.push({ s: s.key, r })
  const v = r ? (r.results || []).filter((x) => x.verdict === 'e2e-verified').length : 0
  log(`e2e场景 ${s.key}: ${v}/${r ? (r.results || []).length : 0} verified`)
}

// ════ Phase 4:收口对账 ════
phase('收口对账')
const verdicts = e2e.filter((x) => x && x.r).flatMap((x) => x.r.results || [])
const collect = await agent(
  `# Round-4 收口:翻台账+三表对账+SESSION DONE\ncwd ${HELIX}(分支 ${BRANCH})。\n` +
  `## e2e裁决(${verdicts.length}条):\n${JSON.stringify(verdicts, null, 1).slice(0, 9000)}\n\n` +
  `任务:\n` +
  `1. Edit ${LEDGER}:按裁决更新每UC(e2e-verified→✅;data-dep→⏭;harness-gap→⬜注明物理够不到;wire-bug→🔴)。叠加Round-3已verified的12个不回退。\n` +
  `2. 跑 bash scripts/fullmap-coverage.sh 记三表;UC新覆盖数。\n` +
  `3. SESSION.md §0 终态行:✅ DONE Round-4 或 ⚠️ PARTIAL(列剩余UC+物理够不到的诚实标注:文件上传/部分data-dep)。\n` +
  `4. cargo test -p helix-im 终验 + git diff -- crates/helix-core/ 空(HX-C001)。commit(docs+scripts scope)。不push。\n` +
  `返回 {三表覆盖数, 台账三态分布, Round-4净增verified的UC, 剩余UC及物理/数据阻因, SESSION DONE行}。`,
  { label: 'collect:round4', phase: '收口对账', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
)

return {
  branch: BRANCH,
  wire_bugs_fixed: ['UC-1.2', 'UC-1.6'],
  e2e_verified: verdicts.filter((v) => v.verdict === 'e2e-verified').map((v) => v.uc),
  remaining: verdicts.filter((v) => v.verdict !== 'e2e-verified').map((v) => ({ uc: v.uc, verdict: v.verdict })),
  collect,
}
