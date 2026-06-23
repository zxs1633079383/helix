export const meta = {
  name: 'helix-http-ws-takeover-design',
  description: '设计 helix-im 全权接管 cses-client HTTP+WS+业务逻辑的长链路方案（前端退化为纯渲染层）',
  phases: [
    { title: 'Scan', detail: '并行读者：后端 HTTP(4域)/WS 事件 + 前端 usecase/UI/tauri-im + helix-im 现状 + IPC 接缝' },
    { title: 'Design', detail: '综合域地图 → 接管架构 + 分阶段 worktree 路线' },
    { title: 'Review', detail: '专家对抗面板：tauri-接缝 / runtime-core 边界 / rust 正确性' },
    { title: 'Finalize', detail: '合并审查 → 最终设计文档 + 阶段表 + 开放决策' },
  ],
}

const HELIX = '/Users/mac28/workspace/rustWorkspace/helix'
const PARTIALS = `${HELIX}/crates/helix-im/docs/csesapi-migration/full-map/partials`
const MIG = `${HELIX}/crates/helix-im/docs/csesapi-migration`
const IMDOCS = `${HELIX}/crates/helix-im/docs`
const MM = '/Users/mac28/workspace/golangProject/mattermost'
const CLIENT = '/Users/mac28/workspace/angular/cses-client'

const IGNORE = '【硬约束】完全忽略文档中任何 bitmap / bitmap segment / bitmap ack / channel_bitmap 相关设计（ADR-013 已下线），不纳入方案。'
const GOAL =
  '【总目标】让 helix-im(Rust) 全权接管 cses-client 的 HTTP 出站 + WS 入站 + 业务处理逻辑；' +
  'Angular 前端退化为纯渲染层（业务零下沉到前端），业务全部下放 Rust；' +
  '前后端经 Tauri IPC（im:__bus__ 单总线事件 + invoke 命令）通信，或论证更优解。' +
  '后端参考真源 = ' + MM + '（mattermost/server/channels/csesapi）；前端 = ' + CLIENT + '。' +
  '注意：helix-im 已落地 引擎三件套(HTTP/WS/Storage) + sync-v2 ~90%（connect/increment/sync/落库/echo对账/im:__bus__信封 已通），' +
  '本方案要在此基础上把"全量 124 HTTP + 19 WS 事件 + 业务编排 + Tauri 接缝"补齐到可替换现网 src-tauri/features/im。'

// ---------- Phase 1: Scan ----------
phase('Scan')

const SCAN = [
  { label: 'be-posts', prompt:
    `读 ${PARTIALS}/1--backend-http-posts.md + ${PARTIALS}/9--gap-posts.md（必要时对照 ${MM}/server/channels/csesapi 真 handler）。` +
    `产出 posts 域(35 HTTP)接管地图：逐接口 路径/方法/请求体关键字段/响应信封/触发的 WS 事件/当前 helix-im 覆盖态(done|partial|missing)/接管还缺什么。标命名陷阱(snake_case/timeStamp 大写 S/postIds 单数)。` },
  { label: 'be-channel', prompt:
    `读 ${PARTIALS}/2--backend-http-channel-sync.md + ${PARTIALS}/10--gap-channel-sync.md。` +
    `产出 channel 域(34 HTTP)接管地图：含 sync wire 硬契约(fromSeq/eventSeq/too_long>5000/events≤500/cursor≤500)、CommonRes 信封、系统消息驱动的 WS 表达、casing quirk。逐接口标覆盖态 + 接管缺口。` },
  { label: 'be-user-misc', prompt:
    `读 ${PARTIALS}/3--backend-http-user-misc.md + ${PARTIALS}/11--gap-user-misc.md。` +
    `产出 user-misc 域(15 HTTP：用户/团队/分组/模块/通知/搜索/健康)接管地图。重点标真阻塞缺失(createUser / users/status/ids 在线状态)与不走信封的 3 端点、search 四件套空桩/data 形态偏离。逐接口覆盖态 + 接管缺口。` },
  { label: 'be-bot-agent', prompt:
    `读 ${PARTIALS}/4--backend-http-bot-agent.md + ${PARTIALS}/12--gap-bot-agent.md。` +
    `产出 bot-agent 域(37 真实 HTTP，排除 3 个 cross-repo demo)接管地图：BotAgentTokenAuth(Bearer mp_)、BotAgentWebhookEvent Pulsar fanout、复用标准 post 事件。逐接口覆盖态 + 接管缺口。` },
  { label: 'be-ws', prompt:
    `读 ${PARTIALS}/5--backend-ws-events.md（对照 ${MM}/server/public/model/websocket_message.go）。` +
    `产出 19 个 WS 推送事件的接管地图：每个事件常量名/data 顶层字段(尤其 event_seq 注入位置)/触发来源 handler/客户端 Rust router.rs 监听的 action 名。` +
    `特别标注 posts 域事件命名重构(post_update → posts_update/post_urgent/post_confirm_urgent)与客户端监听名的双端核对风险。` },
  { label: 'cl-usecases', prompt:
    `读 ${PARTIALS}/6--client-usecases.md。` +
    `产出 message-v3 service 层 42 用例接管地图：双 HTTP 实例(imHttp 自动拼 /api/cses + ImUnwrapStrategy / api AppHttpService)、WS 四段式(服务端 action→Rust router.route→emit_im(im:*)→Angular getXxx$→dispatch)全映射表。` +
    `关键：列出前端"业务逻辑"目前散落在哪些 service 方法里——这些是要下沉到 Rust 的清单。` },
  { label: 'cl-ui-render', prompt:
    `读 ${PARTIALS}/7--client-ui-rendering.md。` +
    `产出 10 用例集 UI 渲染依赖地图：readBits 字符串位图/unReadPostId(大写 R)/messageType.SYSTEM='SYSTEN' 拼写/self-message 本地 id 空串只有 temporaryId/lastEventSeq cursor。` +
    `关键：界定"纯渲染所需的最小字段契约"——前端退化为纯渲染后，Rust 必须经 im:__bus__ 投喂哪些字段才能让 UI 零业务逻辑地渲染。附"迁移服务端字段契约清单 10 项"。` },
  { label: 'cl-tauri-im', prompt:
    `读 ${PARTIALS}/8--client-tauri-im.md + 实地扫 ${CLIENT}/src-tauri/src/features/im/（这是要被 helix 替换的现网 Rust IM 核心）。` +
    `产出 src-tauri Rust IM 14 用例集接管地图：sync/notify 响应外层信封(status/data/entries/serverTime)、entry/ImEvent camelCase 但 createAt、SyncEntryKind 枚举、allHash FNV-1a 64 锚定向量、WS 帧 event_seq 三级 fallback、gate 双路径。` +
    `关键：列出现网 src-tauri/features/im 暴露给前端的 invoke 命令 + emit 事件清单（这是 helix 必须 1:1 兼容的 IPC 表面），以及 helix 替换它的接缝点。` },
  { label: 'helix-current', prompt:
    `读 ${IMDOCS}/sync-v2-能力与缺口状态.md + ${IMDOCS}/csesapi-migration/真机curl真源.md，实地扫 ${HELIX}/crates/helix-im/src/ 与 ${HELIX}/crates/helix-driver-native/src/engine_loop.rs。` +
    `产出 helix-im 现状接管地图：① 已接管什么(connect/hello/increment/sync/落库/echo/im:__bus__/心跳/并发闸)；② outbound 已对齐的 6 个 curl 真源命令体(send/makeTopic/create/revoke/close/member)；③ parser 已认的 WS 帧；④ 当前 module.rs/registry.rs/parser.rs/acl 的 handler 注册结构(扩展点在哪)；⑤ 距离"全量 124 HTTP + 19 WS"还差的接管面。` },
  { label: 'ipc-seam', prompt:
    `实地扫 ${CLIENT}/src/pages/message-v3/（前端 TS）找出前端经 Tauri 与 Rust 通信的全部接缝：` +
    `① 前端 invoke 调用的 Tauri 命令名清单；② 前端 listen 的事件名清单(尤其 im:__bus__ 单总线信封结构 + ImEventBusDispatcher)；③ store:* / imDataSource 隐式契约。` +
    `产出"IPC 接管契约"：helix 接管后必须保持兼容的命令+事件全集，以及哪些当前由前端做的业务可改为 Rust 经 im:__bus__ 推送结果让前端纯渲染。` +
    `评估"沿用现有 IPC vs 更优解(如 Rust 端聚合后单事件投喂/状态快照拉取)"的取舍。` },
]

const maps = (await parallel(SCAN.map(s => () =>
  agent(`${GOAL}\n${IGNORE}\n\n${s.prompt}\n\n输出聚焦"接管"视角的 markdown 地图，控制在要点，不抄全文。`,
    { label: s.label, phase: 'Scan' })
    .then(text => ({ label: s.label, text }))
))).filter(Boolean)

// ---------- Phase 2: Design ----------
phase('Design')
const mapsText = maps.map(m => `### 域地图: ${m.label}\n${m.text}`).join('\n\n---\n\n')

const design = await agent(
  `${GOAL}\n${IGNORE}\n\n` +
  `你拿到 10 份域接管地图（后端 4 域 HTTP + WS 事件 + 前端 usecase/UI/tauri-im + helix-im 现状 + IPC 接缝）。` +
  `也可按需 Read ${HELIX}/crates/helix-im/docs/csesapi-migration/full-map/30-capability-gap-matrix.md（缺口矩阵）+ ${MIG}/02-迁移蓝图.md。\n\n` +
  `${mapsText}\n\n` +
  `## 你的任务：设计 helix-im 全量接管 HTTP+WS 的长链路方案，要求：\n` +
  `1. **接管架构 + 状态机建模**：明确 helix(helix-im + helix-driver-native) 如何嵌入/替换 cses-client 的 src-tauri/features/im；前端如何退化为纯渲染层；IPC 表面(invoke 命令 + im:__bus__ 事件)如何保持兼容或论证更优解。给出数据流全景(前端 invoke → helix-im handle → Effect → driver → HTTP/WS → 入站 → gate → 落库 → im:__bus__ → 前端渲染)。**把整个接管流程显式建模成状态机**：定义连接/同步/发送/接管各自的状态集 + 转移条件 + 触发 Effect；用 mermaid stateDiagram 画出。\n` +
  `2. **分阶段长链路路线(状态机式推进)**：按"域 + 风险分级 + 依赖链"切成有序 Phase，整条路线像状态机一样推进——**每个 Phase 是一个状态节点，前序验收通过(gate)才能转移到下一个**；彼此无影响(文件白名单零交集 + 无数据依赖)的 Phase 标为可并行分支。覆盖全部 124 HTTP(去掉有意不迁的) + 19 WS 事件 + P0 SendPosts 编排 + Tauri 四接缝接入。\n` +
  `3. **每 Phase 必须有验证 gate**：每个 Phase 列 scope 一句话 / 后端参考文件 / helix-im 要实现的文件 / 前端要"掏空业务保留渲染"的文件 / wire 契约要点 / **验收 gate(具体命令 + 通过判据，gate 红则不得转移到下一状态)** / 是否可并行 worktree / 依赖哪些前序 Phase。\n` +
  `4. **测试策略(硬约束：100% 覆盖)**：**每一个 HTTP 接口 + 每一个 WS 事件都必须有对应的测试类/测试用例**(回放剧本 tests/ 风格，参 helix-im/tests/common.rs)；目标**测试覆盖率 100%**——含正常路径 + wire 契约边界(命名陷阱/casing/空信封) + 乱序/too_long/cursor 单调等性质测试。给出测试矩阵(接口/事件 → 测试类名 → 覆盖维度)与覆盖率度量方式(cargo-llvm-cov 或 tarpaulin)。每个 Phase 的验收 gate 必须包含"本 Phase 涉及接口/事件 100% 测试通过 + 覆盖率达标"。\n` +
  `5. **worktree 并行性分析**：哪些 Phase 文件白名单零交集可并行、哪些有依赖必须串行；遵循根 CLAUDE.md §6 worktree 约束。\n` +
  `6. **cses-client 接入方案放最后(本轮只设计不实现)**：helix 全部 Phase ready 后，最后一个 Phase 专门设计 **cses-client 无缝接入方案**——如何用 helix 替换现网 src-tauri/features/im、IPC 兼容层/灰度开关、前端纯渲染改造点、回滚策略、联调验收。**强调：本轮全流程 cses-client 不动代码，只在最后产出接入设计方案。**\n` +
  `7. **开放决策**：列出需要用户拍板的点(如 createUser/在线状态 由 helix 迁 vs ucenter 供给、IPC 沿用 vs 更优解、前端改造范围、灰度开关策略、覆盖率工具选型)。\n\n` +
  `输出完整 markdown 方案。务必落到真实文件路径与 wire 字段，不要泛泛而谈。`,
  { label: 'design-architect', phase: 'Design' })

// ---------- Phase 3: Review (expert adversarial panel) ----------
phase('Review')
const LENSES = [
  { label: 'rv-tauri-seam', agentType: 'rust-tauri-skeleton-dev', focus:
    'IPC 接缝 + 现网对齐(唯一权威)：helix 接管后暴露的 invoke 命令 + im:__bus__ 事件是否与 cses-client 前端现有契约 1:1；前端能否真"零改/仅删业务"地退化为纯渲染；message-v3 service 42 用例的调用面是否被完整覆盖；store:*/imDataSource 隐式契约有没有漏；灰度开关(灰度换引擎)接缝是否合理。' },
  { label: 'rv-core-boundary', agentType: 'rust-runtime-agnostic-core-dev', focus:
    '双端边界 + HX-C001..C013：core 零运行时/零业务 schema、async port Send 边界、features additive、热路径 O(1)、FFI 边界、Effect 批处理、sync-wire cursor 契约、Workflow 改 git ref。方案是否把业务正确放在 helix-im 层而非泄漏进 core/driver；接管全量后是否破坏 sans-IO 确定性。' },
  { label: 'rv-correctness', agentType: 'rust-dev', focus:
    'wire 契约正确性 + 热路径 + 并发：124 HTTP outbound 体 / 19 WS inbound 帧逐域对齐、命名陷阱、cursor 单调不回退、echo 对账、SendPosts 编排正确性、全量接管后 sync 风暴并发闸是否仍够、有无 read-modify-write 热路径退化。' },
]
const reviews = (await parallel(LENSES.map(l => () =>
  agent(`${GOAL}\n${IGNORE}\n\n## 待审方案\n${design}\n\n` +
    `## 你的审查视角\n${l.focus}\n\n` +
    `对抗证伪：找出方案中会导致"接管失败 / 契约漂移 / 架构违规 / 前端无法纯渲染 / worktree 冲突"的**具体**问题(引到 Phase 编号与文件)，每条给可执行修复建议。也指出方案遗漏的接管面。默认从严：拿不准的也列为风险。`,
    { label: l.label, phase: 'Review', agentType: l.agentType })
    .then(text => ({ label: l.label, text }))
))).filter(Boolean)

// ---------- Phase 4: Finalize ----------
phase('Finalize')
const reviewsText = reviews.map(r => `### 审查: ${r.label}\n${r.text}`).join('\n\n---\n\n')

const FINAL_SCHEMA = {
  type: 'object',
  required: ['readme', 'archStateMachine', 'phaseRoadmap', 'testStrategy', 'clientIntegration', 'phases', 'openDecisions', 'reviewVerdict'],
  properties: {
    readme: { type: 'string', description: '文件夹 README.md：总览 + 北极星 + 文件索引 + 状态机一图流' },
    archStateMachine: { type: 'string', description: '01-接管架构与状态机.md：helix 替换 src-tauri/features/im 的架构 + 数据流全景 + mermaid 状态机(连接/同步/发送/接管 状态集+转移+Effect)' },
    phaseRoadmap: { type: 'string', description: '02-分阶段路线.md：状态机式有序 Phase 推进图(mermaid) + 每 Phase(scope/后端参考/helix实现文件/前端掏空文件/wire要点/验收gate/可并行/依赖) + worktree 并行性分析' },
    testStrategy: { type: 'string', description: '03-测试策略.md：每个 HTTP 接口 + 每个 WS 事件 → 测试类/用例 矩阵 + 覆盖维度 + 100% 覆盖率度量方式 + 每 Phase gate 的测试判据' },
    clientIntegration: { type: 'string', description: '04-cses-client接入方案.md：最后一步、仅设计不实现——helix 替换现网 IM 核心、IPC 兼容层/灰度开关、前端纯渲染改造点、回滚、联调验收' },
    reviewVerdict: { type: 'string', description: '一句话：审查发现的最关键 3 个问题是否已在终稿收口' },
    phases: {
      type: 'array',
      description: '分阶段长链路路线，有序（结构化镜像 phaseRoadmap）',
      items: {
        type: 'object',
        required: ['id', 'title', 'scope', 'worktreeable', 'deps', 'gate'],
        properties: {
          id: { type: 'string' },
          title: { type: 'string' },
          scope: { type: 'string' },
          filesBackendRef: { type: 'array', items: { type: 'string' } },
          filesHelixImpl: { type: 'array', items: { type: 'string' } },
          filesFrontendGut: { type: 'array', items: { type: 'string' } },
          interfacesCovered: { type: 'array', items: { type: 'string' }, description: '本 Phase 接管的 HTTP 接口 + WS 事件清单（每个都要有测试类）' },
          gate: { type: 'string', description: '验收 gate：命令 + 通过判据（含 100% 测试 + 覆盖率），gate 红不得转移到下一状态' },
          worktreeable: { type: 'boolean' },
          parallelGroup: { type: 'string', description: '可并行分支编号（无影响的 Phase 同组并行）；串行则留空' },
          deps: { type: 'array', items: { type: 'string' } },
        },
      },
    },
    openDecisions: {
      type: 'array',
      description: '需用户拍板的决策点',
      items: {
        type: 'object',
        required: ['question', 'options', 'recommendation'],
        properties: {
          question: { type: 'string' },
          options: { type: 'array', items: { type: 'string' } },
          recommendation: { type: 'string' },
        },
      },
    },
  },
}

const final = await agent(
  `${GOAL}\n${IGNORE}\n\n## 草案方案\n${design}\n\n## 三份专家审查\n${reviewsText}\n\n` +
  `## 你的任务\n把草案与三份审查合并成**最终设计文档集**(将落到 helix-im/docs/ 新文件夹，分多个 markdown 文件)：吸收审查修复、补齐遗漏接管面、收口最关键风险。硬约束：\n` +
  `- 整条路线像**状态机**：Phase = 状态节点，验收 gate 通过才转移；无影响(白名单零交集+无数据依赖)的 Phase 并行(同 parallelGroup)。\n` +
  `- **每个 HTTP 接口 + 每个 WS 事件都要有测试类/用例，测试覆盖率 100%**；每 Phase gate 含"涉及接口/事件 100% 测试通过 + 覆盖率达标"。\n` +
  `- **cses-client 接入方案是最后一个 Phase，本轮只设计不写代码**；前面所有 Phase 都是 helix 侧实现。\n` +
  `按 schema 各字段产出对应文件正文(readme/archStateMachine/phaseRoadmap/testStrategy/clientIntegration)，并填结构化 phases / openDecisions。`,
  { label: 'finalize', phase: 'Finalize', schema: FINAL_SCHEMA })

return final
