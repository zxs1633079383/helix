export const meta = {
  name: 'cses-client-helix-integ',
  description: 'cses-client 接入 helix 接管引擎 — CI-1 地基(git-dep+灰度) → CI-2 Tauri薄壳 → CI-3 前端接线（灰度可逆，legacy 保留）',
  phases: [{ title: 'ClientInteg', detail: 'CI-1 helix git-dep + IM_ENGINE 脚手架 / CI-2 Tauri 薄壳+总线桥 / CI-3 前端纯渲染接线' }],
}

const CWT = '/Users/mac28/workspace/angular/cses-client/worktrees/helix-integ'
const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const PLAN = `${HELIX}/worktrees/im-takeover/crates/helix-im/docs/takeover-plan`
const HELIX_GITDEP = '{ git = "file:///Users/mac28/workspace/rustWorkspace/helix", branch = "feat/im-takeover" }'

const COMMON =
  `你在 cses-client 接入 worktree 实现「用 helix 接管引擎替换现网 src-tauri/features/im」的一个 Phase（灰度可逆，legacy 路径全程保留）。\n` +
  `工作区（绝对路径）：${CWT}　分支：feat/helix-takeover-integ\n\n` +
  `## 背景\n` +
  `helix-im 已全权接管 IM 的 HTTP+WS+业务（89 outbound + 19 WS + Tauri 四接缝 + im:__bus__ 单总线 + 冻结的 projection-schema）。helix 代码在另一仓 ${HELIX} 的 feat/im-takeover 分支（7d8d91b）。\n` +
  `本接入目标：cses-client 前端退化为纯渲染层，业务全走 helix。**本轮只在 cses-client 仓做接入,不改 helix。**\n\n` +
  `## 必读\n` +
  `- ${PLAN}/04-cses-client接入方案.md（接入设计真源：二进制替换/IPC 兼容/灰度/回滚）+ ${PLAN}/01-接管架构与状态机.md（IPC 兼容两表）。\n` +
  `- ${HELIX}/worktrees/im-takeover/crates/helix-im/docs/projection-schema.md（前端渲染契约：每个 im:__bus__ 事件字段集冻结真源）。\n` +
  `- helix 参照装配：${HELIX}/worktrees/im-takeover/crates/helix-host-cli/src/main.rs（headless 同构孪生——Tauri 薄壳照它装配 ExecutionShell+ImModule+ports+driver，唯一区别是最外层 I/O = IPC 而非 stdin）。\n` +
  `- 现网被替换对象：${CWT}/src-tauri/src/features/im/（commands.rs/handlers/http.rs/message_loop.rs/event_bus.rs/domain）——**保留不删,灰度开关切**。\n\n` +
  `## 铁律\n` +
  `- 【灰度可逆】现网 features/im 全程保留;新 helix 引擎并行装配,由开关 IM_ENGINE(=helix|legacy,env 或 cargo feature) 切;回滚 = 切回 legacy。绝不删现网路径。\n` +
  `- 【helix 依赖】src-tauri 加 helix git-dep：helix-core / helix-im / helix-driver-native = ${HELIX_GITDEP}（指 feat/im-takeover 分支,不指 main/不指 worktree path）。\n` +
  `- 【IPC 兼容】helix 侧暴露的 invoke 命令名 + im:__bus__ 事件名必须与现网前端契约一致（04 文档表 A/B）;前端能渲染的字段以 projection-schema 为准。\n` +
  `- 【不改 helix】本轮 cses-client 仓内作业;若发现 helix 缺接口/字段 → 写 NEED_HELIX_FIX.md 记 gap(不阻塞自己,mock/stub 占位),不去改另一仓。\n` +
  `- 【git/HX-C13】只在 ${CWT} 内 git add+commit 到 feat/helix-takeover-integ;禁 checkout/branch/merge/push;提交前自检 pwd 含 worktrees/helix-integ + 分支==feat/helix-takeover-integ。\n` +
  `- 【提交体】Conventional Commits 中文 + 结构化 body（影响范围/改动影响面/功能改进/验证）。\n\n` +
  `## 验证（autonomous 自跑,如实报）\n` +
  `- Rust：cargo check --manifest-path ${CWT}/src-tauri/Cargo.toml -p cses_client_lib（首次拉 helix git-dep + Tauri 树,冷编译数分钟正常;只 check 不 build bundle）。\n` +
  `- TS（CI-3）：node_modules 缺则先 npm ci;npx tsc --noEmit 或 ng lint 改动文件;太重则类型级自审 + 如实标注「全量 tsc 需主检出环境」。\n` +
  `- gate 绿 = 编译/类型检查过 + 灰度开关切 legacy 时现网行为零变 + 改动可逆。红则如实 gatePassed:false + 卡点,不硬提交假绿。\n`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' }, gatePassed: { type: 'boolean' }, commit: { type: 'string' },
    summary: { type: 'string' }, gateOutput: { type: 'string' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' } },
    needHelixFix: { type: 'string', description: '若发现 helix 侧 gap（缺命令/字段/事件），写在此（NEED_HELIX_FIX.md 摘要）,main 据此回 helix 补' },
  },
}

const PHASES = [
  { id: 'CI-1', title: '地基：helix git-dep + IM_ENGINE 灰度脚手架', detail:
    `1. src-tauri/Cargo.toml [dependencies] 加 helix-core/helix-im/helix-driver-native = ${HELIX_GITDEP}（首次会拉分支冷编译,确认能解析到 89 outbound 的接管代码）。\n` +
    `2. 建灰度开关 IM_ENGINE（推荐 env var 运行时切 + 默认 legacy,零风险）：装配层读开关决定走现网 features/im 还是 helix 引擎。本 Phase 只建脚手架（开关 + 两路装配占位,helix 路可暂 stub），不接全命令。\n` +
    `3. 确认现网构建不破：IM_ENGINE 缺省=legacy 时 cargo check 绿、现网 features/im 零改动。\n` +
    `gate：cargo check -p cses_client_lib 绿（helix git-dep 解析成功）+ legacy 路径零变。` },
  { id: 'CI-2', title: 'Tauri 薄壳：语义命令 invoke→Tick + im:__bus__ 总线桥', detail:
    `照 helix host-cli 装配（headless 孪生）在 src-tauri 建 helix 接缝层（新模块,非改 features/im）：\n` +
    `1. 装配 ExecutionShell + ImModule + ports（NativeStorage/NativeHttp::with_crosscut/NativeTransport/NativeEventSink/Clock）+ 跑泵（复用 helix-driver-native run_engine_loop_with_transports）。身份头/userSnapshot 从现网登录态展开注入 ImConfig（对照 host-cli env 注入点,改从 cses-client 登录态取）。\n` +
    `2. #[tauri::command] 语义命令薄壳全集（im_send_message/im_create_posts/... → Tick::Command）+ 投影命令 im_query_messages_by_channel,经 generate_handler! 注册（IM_ENGINE=helix 时启用）。命令名对齐 04 文档表 A/B + 现网前端 invoke 名。\n` +
    `3. helix NativeEventSink → app.emit("im:__bus__", envelope) 桥接（bus_guard 已在 helix 侧守护;driver 批处理 Vec<Effect>）。\n` +
    `gate：cargo check -p cses_client_lib 绿（helix 接缝层编译过）+ 命令注册全集 + legacy 仍可切回。helix 侧缺口写 needHelixFix 不阻塞。` },
  { id: 'CI-3', title: '前端接线：IM_ENGINE 开关 + 核心收发渲染路径切 helix', detail:
    `**本 Phase 重在建立"前端纯渲染"接线模式,不要求一次掏空全部 42 service**：\n` +
    `1. 前端建 IM_ENGINE 开关读取（与 Rust 侧一致）+ message-v3 数据源切换点：IM_ENGINE=helix 时,核心路径（发消息 im_send_message / 收消息 listen im:__bus__ im:post:received / 上屏 im_query_messages_by_channel / 已读 / 撤回）改调 helix 语义 invoke + 按 projection-schema listen 渲染;legacy 时走现网。\n` +
    `2. 把核心收发渲染链路的业务逻辑标删/下沉（按 projection-schema 前端零补算:信号事件回读本地行渲染）;其余 service 改造列清单进 04 文档的"前端改造清单"节(不一次做完)。\n` +
    `3. 命名陷阱守 projection-schema：unReadPostId 大写R / messageType SYSTEN / createAt。\n` +
    `gate：tsc --noEmit 或 ng lint 改动文件过（node_modules 缺则 npm ci;太重则类型自审+如实标注）+ legacy 路径 UI 零变。helix 侧缺口写 needHelixFix。` },
]

phase('ClientInteg')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红,跳过' }); continue }
  const r = await agent(
    `${COMMON}\n\n## 你的 Phase = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程：Read 设计/契约/host-cli 参照 → 实现（灰度可逆）→ 跑验证 → 绿则结构化提交 → 返回结构化结果。`,
    { label: `integ-${p.id}`, phase: 'ClientInteg', schema: SCHEMA, effort: 'high', agentType: 'cses-client-dev' })
  results.push(r)
  if (!r || !r.gatePassed) { stop = true; log(`⛔ ${p.id} gate 未过 → 停`) }
  else log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}`)
}
return { results, stoppedEarly: stop, branch: 'feat/helix-takeover-integ', worktree: CWT }
