export const meta = {
  name: 'cses-round6-uc-reclaim',
  description: 'Round-6 定向夺回:B类(5.6公告走im:read:result/1.5隔离revoke/2.3·2.4 query族已解封)e2e夺回 + C类(2.1同4.1 increment/5.7 online已观测)证据重标。worktree 隔离。',
  whenToUse: '续 Round-5(UC 29/37·治理已收敛 merge main);在 worktrees/uc-reclaim 隔离跑;只夺可达的(1.3物理/4.3注入gap 不碰)',
  phases: [
    { title: 'B类e2e夺回', detail: '5.6公告list走read-relay + 1.5隔离revoke剧本 + 2.3/2.4 query族(accept已解封)真Go e2e' },
    { title: 'C类证据重标', detail: '2.1切群首屏=4.1同increment证据 + 5.7 online已观测·重标verified-by-proxy' },
    { title: '收口对账', detail: '翻台账+coverage+SESSION·诚实标剩余物理/数据够不到' },
  ],
}

const WT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/uc-reclaim'
const BRANCH = 'feat/cses-round6-uc-reclaim'
const LEDGER = 'docs/review/uc-coverage-ledger.md'
const PERSIST = 'scripts/host-cli-go-chain-e2e-persistent.sh'
const E2E_ENV = `HELIX_COOKIE_ID=444 HELIX_COMPANY_ID=64118eebd2b665246b7880eb MEMBER_USERS_JSON='["678","555"]'`

const DOCTRINE = `
## 🔴 验证口径(已确立·橡皮章零容忍)
- 发送者会收 action=post 自echo(self-omit误诊已纠)。持久连接 ${PERSIST} 整条链1次hello·wait_for回声即返。
- 读族(announcement/list·bookmark)走 read-relay → DomainEvent emit im:read:result bytes>0 = verified(像 9.2 书签那样)。
- 判 verified 必须贴真 RAW WS 帧 或 im:read:result emit bytes>0 或 HTTP200+落库;贴不出如实降级(data-dep/harness-gap)。
- **明确不碰**:1.3 文件上传(host-cli 物理够不到)·4.3 too_long(无 cursor 注入入口)——这俩留 untested 不假判。
- 账号 ${E2E_ENV}。改代码守 HX-C001(git diff -- crates/helix-core/ 空)+ structure-gate exit 0。不push。
`
const E2E_SCHEMA = {
  type: 'object',
  required: ['unit', 'results'],
  properties: {
    unit: { type: 'string' },
    results: {
      type: 'array',
      items: {
        type: 'object',
        required: ['uc', 'verdict', 'evidence'],
        properties: {
          uc: { type: 'string' },
          verdict: { type: 'string', enum: ['e2e-verified', 'verified-by-proxy', 'data-dep', 'harness-gap', 'untested'] },
          evidence: { type: 'string', description: 'RAW WS帧/im:read:result bytes/HTTP code/同机制锚' },
        },
      },
    },
  },
}

// ════ Phase 1:B类 e2e 夺回(串行·共享真Go)════
phase('B类e2e夺回')
const B = [
  { key: '5.6-公告', uc: 'UC-5.6 群公告',
    how: `announcement 读族走 read-relay(像书签 9.2)。在 worktree 跑持久 host-cli:① 自建群 ② cmd im_announcement_list {channelId} → 看 DomainEvent emit im:read:result bytes>0(列表回灌) ③ 若有 announcement/save 写→list 读回看真公告。验法对齐 9.2 书签(emit im:read:result + 可选 curl 落库双证)。announcement/save 若 fire-and-forget 无回声则标 data-dep,list 读族能 emit 即 verified。` },
  { key: '1.5-撤回', uc: 'UC-1.5 撤回',
    how: `隔离 revoke 剧本(不和 makeTopic 纠缠):持久连接 ① send 一条 → wait_for post 抠 THAT postId ② cmd im_revoke {该postId} → wait_for post_update,断言 data.id==该postId 且 revoke/isDelete 标记。隔离 marker 确保 post_update 归因 revoke 非 makeTopic。看到带该 postId 的 post_update 即 verified。` },
  { key: '2.3-postId定位', uc: 'UC-2.3 postId 定位',
    how: `query 族(accepts_tick 已解封·main 已含修)。postId 定位 = 按 anchor postId 加载上下文:持久连接 send 几条建史 → cmd im_load_older_context {channel_id,anchor_post_id:<某条>,limit} 或 im_query_messages_by_channel → 看 DomainEvent emit im:messages:older_loaded/查询投影 bytes>0 含定位锚消息。` },
  { key: '2.4-回复列表', uc: 'UC-2.4 回复列表',
    how: `确认走哪个命令(im_query_messages_by_channel? 还是 posts read 回复族)。grep helix query.rs/outbound 找回复列表命令 → 持久连接触发 → 看 emit/HTTP200。若命令不存在=untested 记清;若 query 族则 accepts 已解封·跑 e2e 看 emit。` },
]
const bRes = []
for (const b of B) {
  const r = await agent(
    `# Round-6 B类夺回 ${b.key}\ncwd ${WT}(分支 ${BRANCH}·worktree 隔离)。真后端已起(tp通·hello洪泛)。\n` +
    `## UC:${b.uc}\n## 跑法:${b.how}\n${DOCTRINE}\n` +
    `先 git rev-parse --abbrev-ref HEAD 确认在 ${BRANCH}。逐 UC 出 verdict + 真证据(RAW帧/im:read:result bytes/HTTP)。` +
    `若需小代码改动(如补 helper)守 HX-C001 + structure-gate + cargo test 绿才 commit(在 ${BRANCH})。贴不出证据如实降级,禁橡皮章。`,
    { label: `reclaim:${b.key}`, phase: 'B类e2e夺回', agentType: 'rust-tauri-skeleton-dev', effort: 'high', schema: E2E_SCHEMA }
  ).catch(() => null)
  bRes.push({ b: b.key, r })
  const v = r ? (r.results || []).filter((x) => x.verdict === 'e2e-verified' || x.verdict === 'verified-by-proxy').length : 0
  log(`B类 ${b.key}: ${v} verified`)
}

// ════ Phase 2:C类证据重标(单agent·只读取证不改码)════
phase('C类证据重标')
const cRes = await agent(
  `# Round-6 C类证据重标\ncwd ${WT}(分支 ${BRANCH})。**只读取证·不改生产码**。\n` +
  `两个 UC 本质已被代理验证,补证据重标:\n` +
  `1. **UC-2.1 切群首屏**:= cold-start increment_channel 机制,和已 verified 的 UC-4.1(hello增量)/UC-4.5(陌生channel)**同一条 increment_channel 入站→emit 路径**。grep/证:切群首屏触发的 increment 与 4.1 是否同 handler(ws/handlers/increment_channel*.rs)+ 同 emit(im:channel:increment)。同则标 verified-by-proxy,贴同机制锚(4.1/4.5 已绿的 RAW 帧 + 同 handler 证据)。\n` +
  `2. **UC-5.7 在线/分组**:status_change online 帧**已被 P0 probe 实观测**(RAW WS action=status_change data.status=online user_id=444)。online 部分标 verified(贴该 RAW 帧);分组态(group)若需数据则该子项留 data-dep。\n` +
  `${DOCTRINE}\n返回每 UC verdict + 证据锚(同机制 handler/已绿UC + RAW帧)。`,
  { label: 'reclaim:C类重标', phase: 'C类证据重标', agentType: 'rust-runtime-agnostic-core-dev', effort: 'high', schema: E2E_SCHEMA }
).catch(() => null)

// ════ Phase 3:收口对账 ════
phase('收口对账')
const verdicts = [
  ...bRes.filter((x) => x && x.r).flatMap((x) => x.r.results || []),
  ...(cRes && cRes.results ? cRes.results : []),
]
const collect = await agent(
  `# Round-6 收口:翻台账+三表对账+SESSION\ncwd ${WT}(分支 ${BRANCH})。\n` +
  `## B类+C类裁决(${verdicts.length}条):\n${JSON.stringify(verdicts, null, 1).slice(0, 7000)}\n\n` +
  `任务:\n` +
  `1. Edit ${LEDGER}:按裁决更新(e2e-verified/verified-by-proxy→✅;data-dep→⏭;贴不出→保持原标)。Round-5 的 29 不回退。\n` +
  `2. 跑 bash scripts/fullmap-coverage.sh 记三表 + UC 新数。\n` +
  `3. SESSION.md §0 终态行:✅/⚠️ PARTIAL(列最终剩余:1.3 文件上传物理 + 4.3 too_long注入gap + 真data-dep)。\n` +
  `4. cargo test -p helix-im + git diff -- crates/helix-core/ 空 + structure-gate exit 0。commit(docs scope·在 ${BRANCH})。不push。\n` +
  `返回 {三表数, Round-6净增, 最终UC覆盖, 最终剩余UC及物理/数据阻因, SESSION行}。`,
  { label: 'collect:round6', phase: '收口对账', agentType: 'rust-tauri-skeleton-dev', effort: 'high' }
).catch(() => null)

return {
  branch: BRANCH,
  reclaimed: verdicts.filter((v) => v.verdict === 'e2e-verified' || v.verdict === 'verified-by-proxy').map((v) => v.uc),
  still_blocked: verdicts.filter((v) => ['data-dep', 'harness-gap', 'untested'].includes(v.verdict)).map((v) => ({ uc: v.uc, verdict: v.verdict })),
  collect,
}
