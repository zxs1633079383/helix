export const meta = {
  name: 'helix-takeover-finalize-docs',
  description: 'F1 gap-1 handler 臂 → F2 createPosts curl 实证 → F3 联调测试矩阵(可执行清单)+方法论md → F4 全量 md/html 熵管理',
  phases: [{ title: 'Finalize', detail: 'gap-1 / createPosts 核对 / 测试矩阵+md / 文档熵管理' }],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const HWT = `${HELIX}/worktrees/im-takeover`            // feat/im-takeover
const CWT = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ' // feat/helix-takeover-integ
const PLAN = `${HWT}/crates/helix-im/docs/takeover-plan`
const MIG = `${HWT}/crates/helix-im/docs/csesapi-migration`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' }, gatePassed: { type: 'boolean' }, commit: { type: 'string' },
    summary: { type: 'string' }, gateOutput: { type: 'string' }, findings: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' } },
  },
}

const GITGUARD = (wt, branch) =>
  `【git/HX-C13】只在 ${wt} 内 git add+commit 到 ${branch};禁 checkout/branch/merge/push;提交前自检 pwd + 分支==${branch}。结构化中文 commit body。`

const PHASES = [
  { id: 'F1', title: 'helix gap-1：补 im_reconnect + im_read_channel handler 臂', detail:
    `在 helix worktree ${HWT}(分支 feat/im-takeover)修 gap-1。\n` +
    `现象:crates/helix-im/src/acl/from_tick.rs::accepts_tick allowlist 含 im_reconnect/im_read_channel,但 module.rs Tick::Command match 无对应臂 → 落 _ => debug no-op。\n` +
    `做:① im_reconnect → 加 handler 臂产 Effect 触发 driver NativeTransport 重连(连接控制,连接生命周期归 driver;参 04 表 B IMWS:reconnect + host-cli reconnect 路径);② im_read_channel → 加 channel 级已读 handler(emit im:channel:read_echo + 字段级 BatchUpdate WHERE PK 推 unread,禁 RMW);**或**若核实它就是 im_post_read 别名,则从 accepts_tick allowlist 删 im_read_channel 避免悬空(二选一,按行为真源 cses-client/src-tauri/src/features/im/ 已读 handler 定夺)。\n` +
    `铁律:helix-core 零运行时 / 确定性 grep 空 / 配回放测试(tests/common.rs 剧本)100% 覆盖新臂 / structure-gate 不抬 baseline(超 300 拆文件)。\n` +
    `gate:cargo test -p helix-im 绿 + cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi --all-targets 绿 + 零运行时 grep 空 + 新逻辑测试覆盖。\n` +
    GITGUARD(HWT, 'feat/im-takeover') +
    `\n注:cses-client 侧薄壳已对齐命令名,本 Phase 只改 helix;cses-client cargo update 拉新留到联调(CI-4)时做,不在此编译 cses-client。` },
  { id: 'F2', title: 'gap-2：createPosts curl 集合实证(post vs posts + casing)', detail:
    `用户记得「曾有一个含 createPosts 请求的 curl 集合」,不确定端点是 post 还是 posts 结尾。任务:翻遍所有可能的 curl/抓包/Apifox 集合实证 createPosts 的**真实端点路径 + body casing**。\n` +
    `搜索范围(grep -ri 'createpost' 不区分大小写):\n` +
    `- ${MIG}/真机curl真源.md(helix,已知 6 curl 但 CP 报告说无 createPosts——再确认)\n` +
    `- ${CWT}/ 与 /Users/mac28/workspace/angular/cses-client/ 全仓(Apifox 导出 / *.http / curl 脚本 / docs)\n` +
    `- ${HELIX} 全仓 docs\n` +
    `- 后端 /Users/mac28/workspace/golangProject/mattermost/server/channels/csesapi/posts.go 路由注册(确认 endpoint 是 /posts/createPosts 还是 /post/createPosts,以及 handler 名 createPost 单数 vs createPosts 复数)\n` +
    `交叉判定:前端 message.service.ts:802 sendRelayMessages → post$('/posts/createPosts', {posts,channelIds})(CP 已实证 camelCase)。把找到的 curl 集合证据 + 后端路由 + 前端调用三方比对,给**确定结论**:端点全路径 / 单复数 / casing。\n` +
    `若与 helix im_create_posts(posts_relay.rs camelCase /posts/createPosts)不符 → 在 ${HWT} 改 posts_relay.rs 对齐 + 测试 + 提交(${GITGUARD(HWT, 'feat/im-takeover')});若一致 → 无需改码,findings 写清证据链 + 把结论补进 真机curl真源.md(若该 curl 集合确实存在但未收录 createPosts,补录)。\n` +
    `gate:结论有三方证据支撑(curl集合/后端路由/前端调用);若改码则 cargo test 绿。` },
  { id: 'F3', title: '联调测试矩阵(可执行清单)+ 方法论 md', detail:
    `产出两份 md(放 ${CWT}/docs/ 联调归属 cses-client 侧):\n` +
    `**A. 联调测试矩阵(可执行测试清单)** —— CI-4 的逐条测试脚本蓝本:\n` +
    `  - 89 outbound 命令逐条:命令名 / HTTP 端点+方法 / 请求体关键字段 / **触发的 WS 推送事件**(写命令)或「无WS·验HTTP响应」(读命令)\n` +
    `  - 每条期望帧断言点:event_seq 顶层(post/posts_update/post_update)、lastPost JSON string(update_channel/increment_channel)、change_channel_approval 无 Status 后缀、createAt 非 createdAt、命名陷阱(top20 snake_case / getUpdatedPosts timeStamp 大写S)\n` +
    `  - 落库 + im:__bus__ 投影验证点(对照 ${HWT}/crates/helix-im/docs/projection-schema.md 冻结契约)\n` +
    `  - **按 8 阶段顺序编排成可勾选 checklist**:①连接 ②increment ③sync ④发消息echo ⑤mutation ⑥channel写族 ⑦纯读族 ⑧bot-agent;每阶段写明前置依赖 + 操作步骤 + 期望结果 + 判据\n` +
    `  - 真源:WS 映射读 ${MIG}/full-map/partials/5--backend-ws-events.md;命令清单读 ${HWT}/crates/helix-im/src/outbound/ + module.rs;契约读 projection-schema.md\n` +
    `**B. 方法论 md** —— 沉淀「HTTP+WS 接管联调方法论」:每个写 HTTP 必有对应 WS 回声、读 HTTP 无 WS、按依赖链状态机式排序测试、event_seq/lastPost/casing 三铁律断言。区别于已有全局 skill gated-phase-migration(那是迁移编排方法论),这篇聚焦**联调验证方法论**(HTTP↔WS 因果 + 测试编排)。\n` +
    `gate:矩阵覆盖全部 89 命令(计数核对)+ 8 阶段齐 + 每写命令的 WS 映射与 5--backend-ws-events.md 一致;两 md 自洽。\n` +
    GITGUARD(CWT, 'feat/helix-takeover-integ') },
  { id: 'F4', title: '全量 md + html 熵管理', detail:
    `对本次接管涉及的全部文档做熵管理(减重 / 同步 / 更新索引 / 去陈旧),**两 worktree 都做**:\n` +
    `范围:${HWT}/crates/helix-im/docs/(takeover-plan/ + csesapi-migration/ + sync-v2 状态) + ${HWT} 根 SESSION.md/各 crate CLAUDE.md;${CWT}/ 的 SESSION.md + NEED_HELIX_FIX.md + docs/ + src/pages/message-v3/docs/。\n` +
    `动作:\n` +
    `1. **更新终态**:takeover-plan 各文档反映最终态(gap-1 已补/createPosts 已实证/89 命令/projection-schema 冻结/CI-1~3 接入完成/CI-4 待 Go);sync-v2 状态卡更新。\n` +
    `2. **收口 transient 文件**:NEED_HELIX_FIX.md 已解的 gap 标 RESOLVED(gap-1)+ 留未解(gap-3);SESSION §0 终态行齐。\n` +
    `3. **CLAUDE.md ↔ AGENTS.md 镜像同步**(structure-gate 镜像闸铁律):两 worktree 内每 crates/*/ 的 CLAUDE.md 改了就同步 AGENTS.md,diff 必须空。\n` +
    `4. **索引更新**:helix-im/CLAUDE.md 文档索引挂上 takeover-plan/ + 新测试矩阵 md;cses-client message-v3/docs/README 挂上 14-helix接管前端改造清单 + 联调矩阵。\n` +
    `5. **html**:grep 两仓 *.html(若有渲染文档/设计稿),陈旧的同步或标注;无则 findings 写明「无 html 文档需更新」。\n` +
    `6. 删重复/陈旧散文,合并同主题碎片(熵减),但**不删有效内容**。\n` +
    `gate:structure-gate 镜像闸 PASS(CLAUDE/AGENTS 一致)+ 索引指向的文件都存在 + 无断链;两 worktree 各自 commit。\n` +
    `两 worktree 分别 git 操作:helix 侧 ${GITGUARD(HWT, 'feat/im-takeover')} ;cses-client 侧 ${GITGUARD(CWT, 'feat/helix-takeover-integ')}。` },
]

phase('Finalize')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红,跳过' }); continue }
  const r = await agent(
    `你执行 helix 接管收尾的一个任务。背景:helix-im 已全权接管 cses-client IM(89 outbound + 19 WS + Tauri 接缝 + projection-schema 冻结);cses-client 已接入(CI-1~3 灰度可逆)。helix 代码在 ${HWT}(feat/im-takeover),cses-client 接入在 ${CWT}(feat/helix-takeover-integ)。\n\n` +
    `## 你的任务 = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `完成判据:gate 全绿才提交;红则如实 gatePassed:false + 卡点。`,
    { label: `fin-${p.id}`, phase: 'Finalize', schema: SCHEMA, effort: 'high' })
  results.push(r)
  if (!r || !r.gatePassed) { stop = true; log(`⛔ ${p.id} gate 未过 → 停`) }
  else log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}`)
}
return { results, stoppedEarly: stop }
