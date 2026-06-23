export const meta = {
  name: 'helix-takeover-impl-foundation',
  description: 'helix-im 接管实现 — 串行基线 P0→P1→P1.5→P2（gate 绿才转移，红则停）',
  phases: [{ title: 'Foundation', detail: 'P0 基线 / P1 SendPosts / P1.5 出站横切 / P2 outbound-registry' }],
}

const WT = '/Users/mac28/workspace/rustWorkspace/helix/worktrees/im-takeover'
const DOCS = `${WT}/crates/helix-im/docs/takeover-plan`

const COMMON =
  `你在集成 worktree 实现 helix-im「全权接管 cses-client HTTP+WS+业务」方案的一个 Phase。\n` +
  `工作区（绝对路径，所有编辑/命令针对它）：${WT}　分支：feat/im-takeover\n\n` +
  `## 开工先 Read 规格（渐进式披露，按需深读）\n` +
  `- ${DOCS}/02-分阶段路线.md —— 找到你这个 Phase 的小节，里面有 scope/后端参考文件/helix 实现文件/前端掏空文件/wire 契约/验收 gate/依赖。\n` +
  `- ${DOCS}/03-测试策略.md —— 你这个 Phase 涉及的接口/事件的测试矩阵 + 覆盖维度（每个接口/事件都要有测试类，目标 100% 行覆盖）。\n` +
  `- ${DOCS}/01-接管架构与状态机.md —— 架构与边界约束（投影 payload schema / Clock·IdSource 注入 / IPC 兼容两表）。\n` +
  `- 行为真源（按需）：/Users/mac28/workspace/angular/cses-client/src-tauri/src/features/im/ 对应 handler；` +
  `${WT}/crates/helix-im/docs/csesapi-migration/真机curl真源.md（改任何 outbound 体前必读，真抓包 > 源码推断）；` +
  `后端 /Users/mac28/workspace/golangProject/mattermost/server/channels/csesapi/。\n\n` +
  `## 铁律\n` +
  `- 【HX-C001】helix-core 零运行时：不在 core import tokio/sqlx/reqwest/web-sys/tauri；业务编排放 helix-im；鉴权 token（含 Bearer mp_）注入留 driver。\n` +
  `- 【HX-C002/确定性】时间经 Clock::now_ms()、id 经 IdSource::next_id()；helix-im/src 内 grep \`SystemTime|Instant::now|Utc::now\` 必空。\n` +
  `- 【bitmap 禁区】完全不碰 bitmap/segment/channel_bitmap（ADR-013 已下线）。\n` +
  `- 【测试】每个接口/事件配测试类（回放剧本风格，参 ${WT}/crates/helix-im/tests/common.rs）；含正常路径 + wire 命名陷阱/casing/空信封边界 + 乱序/cursor 单调性质测试。\n` +
  `- 【编译纪律】中途靠 rust-analyzer 式自审，禁 cargo check 循环；仅在 Phase 完成提交前跑一次完整 gate。\n` +
  `- 【git / HX-C013】只在 ${WT} 内 \`git add\` + \`git commit\` 到 feat/im-takeover；禁 checkout/branch/merge/push。提交前自检：pwd 含 worktrees/im-takeover 且 \`git -C ${WT} rev-parse --abbrev-ref HEAD\` == feat/im-takeover。\n` +
  `- 【命令绝对路径】\n` +
  `    cargo test --manifest-path ${WT}/Cargo.toml -p helix-im\n` +
  `    cargo check --manifest-path ${WT}/Cargo.toml -p helix-core -p helix-im -p helix-driver-native\n` +
  `    cargo tree --manifest-path ${WT}/Cargo.toml -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'  # 必空\n` +
  `- 【提交体】§6 结构化 5 段（影响范围/改动影响面/功能改进/Harness 更新/验证）。scope 标 Phase 号。\n\n` +
  `## 完成判据（状态机转移门）\n` +
  `gate 全绿才算完成 = cargo test 全过 + cargo check 四 crate 过 + core 零运行时 grep 空 + 文档列的 fail-hard 断言全过 + 本 Phase 接口/事件 100% 覆盖。\n` +
  `**gate 红绝不硬提交假绿**：如实返回 gatePassed:false + 卡点。拿不准的边界按"行为真源"实证，不臆造。\n`

const SCHEMA = {
  type: 'object',
  required: ['phase', 'gatePassed', 'summary'],
  properties: {
    phase: { type: 'string' },
    gatePassed: { type: 'boolean' },
    commit: { type: 'string', description: 'gate 绿时的 commit sha；红则空' },
    summary: { type: 'string' },
    gateOutput: { type: 'string', description: 'cargo test/check + 覆盖率 关键输出摘要' },
    coverage: { type: 'string', description: '本 Phase 新增文件行覆盖率' },
    filesChanged: { type: 'array', items: { type: 'string' } },
    blockers: { type: 'array', items: { type: 'string' }, description: '卡点 / 需 main 裁决项（如 createPosts PascalCase 真抓包、userSnapshot ucenter 依赖）' },
  },
}

const PHASES = [
  { id: 'P0', title: '基线锁定（无业务代码）', detail:
    '锁 sync-v2 ~90% 为 baseline。建 golden fixtures 工具链（tests/fixtures/ 补真 Go 帧占位 + HELIX_DUMP_FRAMES 抓帧路径）+ 覆盖率脚本 scripts/coverage.sh（cargo-llvm-cov）。' +
    'gate：cargo test -p helix-im 全绿；cargo llvm-cov 跑通并把摘要写 docs/review/coverage-baseline.md（若环境无 llvm-cov，先 cargo install cargo-llvm-cov 或退化记录命令+TODO 但 cargo test 必须真绿）。无新业务逻辑。' },
  { id: 'P1', title: 'SendPosts 编排下沉（最高风险串行根）', detail:
    'messageSend.service.ts 全套编排搬进 helix-im：新 src/outbound/send_build.rs（buildMessageObject：viewers 空补[\'all\']、findReplyRootId、userSnapshot 组装、snapshotToPlainText(2000)、空消息 trim 拦截）+ module.rs::handle_send_message 扩字段 + pending_send.rs connectionId 维度去重。' +
    '回声 action=`post`(非 posted)、`createAt`(非 createdAt)、temporaryId 主键全程不变。完整规格见 02 文档 P1 节，测试见 03。userSnapshot 若有 ucenter 跨 feature 读依赖→记 blockers 不阻塞自身（mock 占位）。' },
  { id: 'P1.5', title: '出站横切层迁移（鉴权/401/断网/trace）', detail:
    '现网 networkInterceptor 的 鉴权头注入/401→logout/断网提示/traceparent 出站透传 全迁 helix-driver-native reqwest 侧（http_manager.rs 的 Arc<RwLock<HeaderMap>>）；core 侧 HttpRequest 只标鉴权意图 X-Auth-Kind:bot|session 不拼 token；失败码 Effect::Emit(im:http:unauthorized / im:net:offline)。完整规格见 02 文档 P1.5 节。' },
  { id: 'P2', title: 'outbound registry 化 + posts 写族', detail:
    '先把 commands.rs::is_outbound 单 match 改 linkme distributed_slice registry（src/outbound/registry.rs，对齐 ws/handlers pattern）；再补 posts 写族 outbound（urgentPost/urgentConfirm/urgentCancel/quickReply/updatePostProps/templateReceived/revoke，每个独立语义命令 + 独立注册文件）。createPosts PascalCase 移出挂 blockers（待真抓包）。完整规格见 02 文档 P2 节。' },
]

phase('Foundation')
let stop = false
const results = []
for (const p of PHASES) {
  if (stop) { results.push({ phase: p.id, gatePassed: false, summary: '前序 gate 红，跳过（状态机不转移）' }); continue }
  const r = await agent(
    `${COMMON}\n\n## 你的 Phase = ${p.id} — ${p.title}\n${p.detail}\n\n` +
    `流程：Read 规格 → 实现（helix 实现文件 + 每接口/事件测试类）→ 跑完整 gate → gate 绿则结构化提交 → 返回结构化结果。`,
    { label: `impl-${p.id}`, phase: 'Foundation', schema: SCHEMA, effort: 'high' })
  results.push(r)
  if (!r || !r.gatePassed) {
    stop = true
    log(`⛔ ${p.id} gate 未过/agent 失败 → 停在此 Phase，状态机不转移（main 介入）`)
  } else {
    log(`✅ ${p.id} gate 绿 @ ${r.commit || '?'}（cov: ${r.coverage || 'n/a'}）`)
  }
}
return { results, stoppedEarly: stop, branch: 'feat/im-takeover', worktree: WT }
