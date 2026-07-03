export const meta = {
  name: 'helix-ci4-e2e-verify',
  description: 'CI-4 真连 Go e2e 验证 — V5 mutation → V6 channel写族 → V7 纯读族 → V8 bot-agent（host-cli 双端日志驱动，顺序跑防 WS 冲突）',
  phases: [{ title: 'CI4-e2e', detail: 'V5 已读/撤回 / V6 昵称/角色 / V7 读族 / V8 bot-agent' }],
}

const HWT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/im-takeover'
const GOLOG = '/Users/mac28/workspace/mmlog/mattermost.log'
const MATRIX = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ/docs/2026-06-23-helix-takeover-联调测试矩阵-CI4.md'
const CH = 'o1guyhbectrwjreiknqn9tp4wo'  // 群主 444 的真频道（increment 落库验证过）

const COMMON =
  `你在真连 Go 后端做 helix 接管的 CI-4 e2e 联调验证的一个阶段。**autonomous 模式,自跑自证,如实报。**\n\n` +
  `## 环境(已就绪)\n` +
  `- Go mattermost :8065 在线(grpc 已验通),日志 ${GOLOG}\n` +
  `- host-cli 工作区 ${HWT}(分支 feat/im-takeover),先 \`cargo build -p helix-host-cli --quiet\`(已 warm)\n` +
  `- 账号:群主 HELIX_COOKIE_ID=444 / 用户 678·555 / HELIX_COMPANY_ID=64118eebd2b665246b7880eb / deviceId 走 ADR-011 兜底(grpc 已通)\n` +
  `- 真频道靶:${CH}\n\n` +
  `## 必读脚手架(双端日志机制)\n` +
  `- ${HWT}/scripts/dev-loop.sh(三层取证:① helix Effect/emit ② Go 日志 delta ③ helix 收帧)\n` +
  `- ${HWT}/scripts/host-cli-session.sh(drive() 逐行喂 host-cli + settle 等回推的范式)\n` +
  `- helix-loop-engine skill 方法论:跑→三层取证→定位失败层→修helix/报Go→复验\n` +
  `- 测试矩阵(本阶段命令↔HTTP↔WS↔断言点真源):${MATRIX}\n\n` +
  `## 实战铁律(已踩坑,务必照做)\n` +
  `1. **stdin 必须保活**:host-cli 读 stdin 到 EOF 即 quit。后台/管道下 stdin 立即 EOF → host-cli 秒退、WS 刚连就断。` +
  `必须用 \`{ sleep 4; echo "<命令>"; sleep 6; ...; } | timeout 30 ./target/debug/helix-host-cli > LOG 2>&1\`——首个 sleep 等 hello 握手盖 connectionId,命令间 settle 等 WS 回推。\n` +
  `2. **HELIX_DUMP_FRAMES=1** 才打印 \`RAW WS inbound ... action=...\` 收帧。RUST_LOG='info,helix_im=debug,helix_driver_host=debug'。\n` +
  `3. **DB 路径 quirk**:host-cli 用 \`sqlite:{path}?mode=rwc\`,真实文件名literally含 \`?mode=rwc\`。设 HELIX_DB=/tmp/ci4_<phase>.db,查时用 \`sqlite3 "/tmp/ci4_<phase>.db?mode=rwc" "..."\`。表名单数:channel/message/channel_event_cursor。\n` +
  `4. **三层取证缺一不可**:① helix 吐了预期 Effect/emit?② Go 日志收到请求 + 无 error/grpc fail?③ helix 收到预期 WS action 帧?三层对齐才算证。\n` +
  `5. **不改 Go**(ADR-008);失败定位到 helix 层可修(白名单内 helix-im),定位到 Go 层则精确报 file:line 给主对话。\n` +
  `6. **安全**:非破坏操作默认跑;**建群/解散群等破坏性操作默认跳过**(除非明确给可销毁靶),跳过的如实记 skipped + 原因。测试消息用清晰标记 \`ci4-<phase>-<ts>\`。\n` +
  `7. 列序读 DB 用 \`sqlite3 -header -column\` 防读错列。\n\n` +
  `## 完成判据\n` +
  `每条命令给 PASS/FAIL/SKIP + 三层证据(helix emit·Go 日志·WS 帧·DB 落库)。失败如实报在哪层 + 根因假设。不假装跑通。`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'allPassed', 'summary', 'cases'],
  properties: {
    phase: { type: 'string' },
    allPassed: { type: 'boolean', description: '本阶段所有可跑命令是否都 PASS(SKIP 不算 fail)' },
    summary: { type: 'string' },
    cases: { type: 'array', description: '逐命令结果', items: {
      type: 'object', required: ['cmd', 'verdict'], properties: {
        cmd: { type: 'string' }, verdict: { type: 'string', description: 'PASS/FAIL/SKIP' },
        wsFrame: { type: 'string', description: '收到的 WS action 帧' },
        evidence: { type: 'string', description: 'helix emit / Go 日志 / DB 落库 证据' },
        note: { type: 'string' },
      } } },
    blockers: { type: 'array', items: { type: 'string' }, description: 'Go 侧问题 / 需 host-cli repl 扩展 / 需特殊配置' },
    helixFixes: { type: 'string', description: '若定位到 helix bug 并修了,写改动 + 复验' },
  },
}

const PHASES = [
  { id: 'V5', title: 'mutation:已读 + 撤回', detail:
    `测 CI-4⑤(矩阵 §2 阶段⑤)。host-cli repl 支持 read/revoke。\n` +
    `- **已读**:\`read ${CH}\` → 期望收 post_read 帧(+ 可能 update_channel 刷未读)+ emit im:post:read + DB message.read_bits 更新。\n` +
    `- **撤回**:先 \`send ${CH} ci4-v5-<ts>\` 拿到 server postId(看 post 回声帧的 id),再 \`revoke <postId>\` → 期望收 posts_update 帧(含被撤 post)+ update_channel 刷 lastPost + emit im:post:updated/deleted + DB revoke 标记。\n` +
    `- 编辑(updatePost)host-cli repl 无对应命令 → SKIP 标注(需 repl 扩展或 curl)。\n` +
    `三层取证:helix 日志 emit + Go delta(publishRead/RevokePost)+ WS 帧 + DB。` },
  { id: 'V6', title: 'channel 写族:昵称 + 角色(非破坏)', detail:
    `测 CI-4⑥(矩阵 §2 阶段⑥)非破坏部分。host-cli repl 支持 nickname/member/close/create。\n` +
    `- **群昵称**:\`nickname ${CH} ci4昵称<ts>\` → 期望收 update_channel_member_nickName 帧(data:{channelId,userId,nickName})+ emit。\n` +
    `- **成员角色/notify**:用 \`member <rawJSON>\`(需现网成员对象 + channelId + role/notify);拿不准真 body 则按矩阵/真机curl真源 构造,失败记 Go 层证据。\n` +
    `- **建群 create / 解散 close = 破坏性 → 默认 SKIP**(记 skipped + 原因:无可销毁靶;如要测需主对话给 CREATE_BODY/CLOSE_CHANNEL_ID)。\n` +
    `三层取证 + change_channel_approval 若测到验「wire 值无 Status 后缀」。` },
  { id: 'V7', title: '纯读族:posts读 / user-misc(无 WS·验 HTTP 响应)', detail:
    `测 CI-4⑦(矩阵 §1.4/§1.5)。**注意**:host-cli stdin repl **不暴露**读族命令(getReplies/top20/list 等),无法经 host-cli 驱动。\n` +
    `策略:\n` +
    `1. 选代表性读端点(如 posts/top20、posts/get、user/list、modules/getAll),用 **curl 直打**(带 cookieId:444 + companyId 头)验 Go 响应 200 + body 形态对齐矩阵 wire 契约(命名陷阱:top20 snake_case channel_id / getUpdatedPosts timeStamp 大写S / health 裸信封 / search channelDetails 第四 key)。\n` +
    `2. 旁证:helix-im 侧这些命令的 outbound build 已被单测覆盖(\`cargo test -p helix-im\` 的 p3a_posts_read_test / p5a_user_misc_test),报告引用。\n` +
    `3. 结论:读族 = build 单测绿 + 代表端点 curl 验 Go 响应形态;**完整 host-cli 往返需 repl 扩展**(记 blocker)。\n` +
    `不臆造,curl 真打真验。` },
  { id: 'V8', title: 'bot-agent:可测性勘验 + 能跑的跑', detail:
    `测 CI-4⑧(矩阵 §1.6)。bot-agent 35 命令多需 bot 配置/mp_ token/agent 后端。\n` +
    `1. 先勘验:Go 后端是否有可用 bot 配置/agent(grep Go 日志 bot/agent;或 curl bot 列表端点看是否有数据)。\n` +
    `2. 能跑的跑(如 createBot 若非破坏/可清理、bot 列表读);需 mp_ token/agent 后端的记 SKIP + 所需配置。\n` +
    `3. 旁证:bot_agent outbound build 单测(p5b_bot_agent_test)已绿。\n` +
    `4. BotAgentWebhookEvent Pulsar fanout(P1-3)需后端支持,记 blocker。\n` +
    `如实报哪些可测、哪些需配置。` },
]

phase('CI4-e2e')
const results = []
for (const p of PHASES) {
  const r = await agent(
    `${COMMON}\n\n## 你的阶段 = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程:Read 脚手架(dev-loop.sh/host-cli-session.sh/host-cli-FULL-uc-e2e.sh)+ 矩阵本阶段节 → 驱动(stdin 保活)→ 三层取证 → 逐命令判 PASS/FAIL/SKIP + 证据 → 返回结构化结果。`,
    { label: `e2e-${p.id}`, phase: 'CI4-e2e', schema: SCHEMA, effort: 'high' })
  results.push(r)
  if (r) log(`${r.allPassed ? '✅' : '⚠️'} ${p.id}: ${(r.cases || []).map(c => c.verdict).join('/')}`)
  // 不 stop——各阶段独立,一个有 SKIP/FAIL 不阻断后续(联调要尽量多覆盖)
}
return { results }
