export const meta = {
  name: 'helix-takeover-impl-g1p4',
  description: 'helix-im 接管实现 — P3a 读族 → P3b channel写族+系统消息 → P4 WS19事件+SyncScheduler（顺序跑集成 worktree，gate 绿才转移）',
  phases: [{ title: 'G1+P4', detail: 'P3a posts读族 / P3b channel写+P0-2系统消息 / P4 WS全量+SyncScheduler+dead-action清理' }],
}

const WT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/im-takeover'
const DOCS = `${WT}/crates/helix-im/docs/takeover-plan`

const COMMON =
  `你在集成 worktree 实现 helix-im「全权接管 cses-client HTTP+WS+业务」方案的一个 Phase。\n` +
  `工作区（绝对路径，所有编辑/命令针对它）：${WT}　分支：feat/im-takeover\n\n` +
  `## 开工先 Read 规格（渐进式披露）\n` +
  `- ${DOCS}/02-分阶段路线.md —— 找你这个 Phase 小节（scope/后端参考/helix实现文件/前端掏空/wire契约/验收gate/依赖）。\n` +
  `- ${DOCS}/03-测试策略.md —— 你涉及的接口/事件测试矩阵（每个接口/事件都要测试类，目标 100% 行覆盖）。\n` +
  `- ${DOCS}/01-接管架构与状态机.md —— 架构边界（投影 payload schema / Clock·IdSource 注入 / IPC 兼容两表）。\n` +
  `- 行为真源（按需）：/Users/mac28/workspace/angular/cses-client/src-tauri/src/features/im/；` +
  `${WT}/crates/helix-im/docs/csesapi-migration/真机curl真源.md（改 outbound 体前必读）；` +
  `后端 /Users/mac28/workspace/golangProject/mattermost/server/channels/csesapi/ 与 server/public/model/websocket_message.go。\n\n` +
  `## 铁律\n` +
  `- 【HX-C001】helix-core 零运行时；业务编排放 helix-im；鉴权 token 注入留 driver。\n` +
  `- 【确定性 C3】时间经 Clock::now_ms()、id 经 IdSource::next_id() 或前端透传；helix-im/src grep \`SystemTime|Instant::now|Utc::now\` 必空。\n` +
  `- 【bitmap 禁区】完全不碰 bitmap/segment/channel_bitmap（ADR-013）。\n` +
  `- 【热路径 HX-C005】unread/lastPost/cursor 写走字段级 BatchUpdate WHERE PK，禁 read-modify-write。\n` +
  `- 【确定性有序 HX-C010】集合迭代→有序 Effect 前先 sort（防 SipHash 随机序）。\n` +
  `- 【outbound 注册】新命令用 linkme distributed_slice（P2 已建 src/outbound/registry.rs），各加独立注册文件，不再共改 commands.rs match。\n` +
  `- 【测试】每接口/事件配测试类（回放剧本 ${WT}/crates/helix-im/tests/common.rs）；含命名陷阱/casing/空信封 + 乱序/cursor 单调性质测试；用从 mattermost 抓的真 Go fixture 比对，不自造 fixture（C5）。\n` +
  `- 【编译纪律】中途 rust-analyzer 自审，禁 cargo check 循环；仅 Phase 完成提交前跑一次 gate。\n` +
  `- 【git/HX-C013】只在 ${WT} 内 git add+commit 到 feat/im-takeover；禁 checkout/branch/merge/push。提交前自检 pwd 含 worktrees/im-takeover 且分支==feat/im-takeover。\n` +
  `- 【命令】cargo test --manifest-path ${WT}/Cargo.toml -p helix-im ；cargo check --manifest-path ${WT}/Cargo.toml -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi ；` +
  `cargo tree --manifest-path ${WT}/Cargo.toml -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'（必空）。覆盖率 ${WT}/scripts/coverage.sh --phase <新文件>。\n` +
  `- 【提交体】§6 结构化 5 段，scope 标 Phase 号。\n` +
  `- 【structure-gate】src/**.rs ≤300 行（存量超限走 baseline 只减不增）；新文件别超 300。\n\n` +
  `## 完成判据（状态机转移门）\n` +
  `gate 全绿才算完成 = cargo test 全过 + cargo check 四 crate 过 + core 零运行时 grep 空 + 确定性 grep 空 + 文档列的 fail-hard 断言全过 + 本 Phase 接口/事件新文件 100% 行覆盖（cfg(test) 块漏行可排除，生产码须 100%）。\n` +
  `gate 红绝不硬提交假绿：如实返回 gatePassed:false + 卡点。拿不准的边界按行为真源实证，不臆造。\n`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' },
    gatePassed: { type: 'boolean' },
    commit: { type: 'string' },
    summary: { type: 'string' },
    gateOutput: { type: 'string' },
    coverage: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' } },
  },
}

const PHASES = [
  { id: 'P3a', title: 'posts 读族 outbound', detail:
    '纯读命令族（无 WS）：getSchedule/postContext/top20/get/getPostsAfterIndex/getReplies/getReplyBranch/queryTodoList/getUpdatedPosts/getLatestPost + bookmark(create/delete/load) + announcement 读(acceptList/list/detail) + post/list（已读位历史读，非 bitmap）。' +
    '新 src/outbound/posts_read.rs（独立 distributed_slice 注册，零交集不碰 channel 文件）；Effect::Http{corr} 回报后 emit 投影（payload 遵 projection-schema / 01 文档）。' +
    '命名陷阱（decode 静默失败防线）：top20→snake_case channel_id；getPostsAfterIndex→postIds(单 id)；getUpdatedPosts→timeStamp(大写S)；时间 createAt。完整规格见 02 文档 P3a + 03 测试矩阵。' },
  { id: 'P3b', title: 'channel 写族 + P0-2 系统消息消费编排', detail:
    'channel create/createSpecifyOwner/close/member(change/leave/role/notify/nickname)/change(info/source/picture/notice/top) outbound + makeTopic 补字段(rootId/users[]/picture/forceCreate) + P0-2 消费编排(total_msg_count 计数 / posts_update 批帧 / reply-chain echo / push fanout)。' +
    'src/outbound/channel_*.rs（独立 distributed_slice 注册）+ channel_write/* 扩系统消息落库；channel handler 只触发不自拼 post 帧；unread/lastPost 字段级 BatchUpdate WHERE PK 禁 RMW。' +
    'wire quirk：load/increment IncrementChannelParam.ChannelId(PascalCase)；create direct_key(snake_case)；member/change/notify payload key=id；#15 change/props 故意不广播勿补 WS。完整规格见 02 文档 P3b + 03。' },
  { id: 'P4', title: 'WS 全量 19 事件 + SyncScheduler + parser 边界 + dead-action 清理', detail:
    '补 router 未注册 handler(post_pin/channel_member_role_updated/update_channel_notice/change_channel_approval/quit_company，各 1 个 distributed_slice 注册文件零改 registry)；parser event_seq 三级 fallback(data.event_seq→props.channel_event_seq→顶层→data.seq)；内存构造 post(bot#2/#32)不推 cursor 边界；' +
    '新 src/sync_scheduler.rs（全局 sync 并发窗口 pending_sync VecDeque + 在途上限 K，解 257 channel 重连风暴）；多端 self echo connectionId 归属；connectionId hello 回注链。' +
    '**并解决 P1 推迟项**：清理 legacy ws/handlers/posted.rs 的 dead-action reconcile（真源 echo 只走 post，posted 是 dead action）——按 02 文档 P4「不要实现 dead action posted/post_urgent/post_edited」，调整既有 echo_reconcile_test 锚到 post 路径。' +
    'wire：仅 post/posts_update/post_update 顶层带 event_seq；update_channel.lastPost 必须 JSON string；change_channel_approval wire 值无 Status 后缀；cursor 锚 nextSeq 非 max event.seq。完整规格见 02 文档 P4 + 03（含 p4_sync_scheduler_test 在途≤K 证伪性质 + cursor 锚 nextSeq 性质）。' },
]

phase('G1+P4')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红，跳过' }); continue }
  const r = await agent(
    `${COMMON}\n\n## 你的 Phase = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程：Read 规格 → 实现（helix 文件 + 每接口/事件测试类）→ 跑完整 gate → gate 绿则结构化提交 → 返回结构化结果。`,
    { label: `impl-${p.id}`, phase: 'G1+P4', schema: SCHEMA, effort: 'high' })
  results.push(r)
  if (!r || !r.gatePassed) { stop = true; log(`⛔ ${p.id} gate 未过 → 停，状态机不转移`) }
  else log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}（cov: ${r.coverage || 'n/a'}）`)
}
return { results, stoppedEarly: stop, branch: 'feat/im-takeover', worktree: WT }
