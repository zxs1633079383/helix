export const meta = {
  name: 'host-cli-go-e2e',
  description: 'host-cli 真连 Go 跑 13-action e2e（驱动 scripts/host-cli-go-e2e.sh）+ 结构化 PASS/FAIL + 失败诊断',
  whenToUse: 'Go(:8065)+grpc up 后，验证共享 driver-host I/O 栈 + 全 WS action wire 契约。args 传 channelId/身份。',
  phases: [
    { title: 'Drive', detail: '跑 host-cli-go-e2e.sh，逐 action 真连 Go，解析结果表' },
    { title: 'Diagnose', detail: '仅失败时：读日志定位根因（未连/未握手/Go 报错/命令未达）' },
  ],
}

// args: { channelId(必), cookieId(必), companyId, closeChannelId, createBody, wsUrl, apiBase, settleMs }
// ⚠️ args 以 JSON 字符串注入（README §约束 2026-06-12 踩坑）→ 归一化 shim 必抄。
const a = typeof args === 'string' ? JSON.parse(args || '{}') : args || {}
if (!a.channelId || !a.cookieId) {
  log('⛔ 缺 args.channelId 或 args.cookieId —— 需真 channelId + userId 身份头')
  return { error: 'missing args.channelId/cookieId', usage: 'Workflow({name:"host-cli-go-e2e", args:{channelId,cookieId,companyId,closeChannelId?,createBody?}})' }
}

// env 前缀（脚本读 env；可选项缺省不设）。
const envPairs = [
  ['CHANNEL_ID', a.channelId],
  ['HELIX_COOKIE_ID', a.cookieId],
  ['HELIX_COMPANY_ID', a.companyId],
  ['CLOSE_CHANNEL_ID', a.closeChannelId],
  ['CREATE_BODY', a.createBody],
  ['HELIX_WS_URL', a.wsUrl],
  ['HELIX_API_BASE', a.apiBase],
  ['SETTLE_MS', a.settleMs],
].filter(([, v]) => v != null && v !== '')
// 单引号包值防 shell 注入/空格（createBody 是 JSON）。
const envPrefix = envPairs.map(([k, v]) => `${k}='${String(v).replace(/'/g, `'\\''`)}'`).join(' ')

const RESULT_SCHEMA = {
  type: 'object',
  required: ['exitCode', 'pass', 'fail', 'skip', 'rows', 'logPath'],
  properties: {
    exitCode: { type: 'integer' },
    pass: { type: 'integer' },
    fail: { type: 'integer' },
    skip: { type: 'integer' },
    logPath: { type: 'string', description: '脚本输出的日志绝对路径（末行「日志 …」）' },
    rows: {
      type: 'array',
      items: {
        type: 'object',
        required: ['status', 'label'],
        properties: {
          status: { enum: ['PASS', 'FAIL', 'SKIP'] },
          label: { type: 'string' },
          note: { type: 'string' },
        },
      },
    },
  },
}

phase('Drive')
const drive = await agent(
  `在仓库根跑 host-cli↔Go 13-action e2e 驱动脚本，**只运行、不改任何文件**：

\`\`\`bash
${envPrefix} bash scripts/host-cli-go-e2e.sh
\`\`\`

脚本会前置检查 Go 可达性（不可达直接 exit 2，未跑任何命令）、逐 action 真连 Go、最后打印 PASS/FAIL/SKIP 表 + 末行「日志 <path>」。
解析它的 stdout：① 退出码 ② 表格每行（✅PASS/❌FAIL/⏭SKIP + label + note）③ 末行「PASS x · FAIL y · SKIP z」④ 日志绝对路径。
按 schema 返回结构化结果。**禁止**自己改脚本/源码/补跑别的命令——只忠实运行 + 解析。`,
  { label: 'drive:e2e', phase: 'Drive', schema: RESULT_SCHEMA, agentType: 'general-purpose' },
)

if (!drive) return { error: 'drive agent 失败（脚本未跑或解析失败）' }
if (drive.exitCode === 2) {
  log('⛔ 前置不满足（Go 不可达 / 缺身份）——未跑任何 action')
  return { verdict: 'PRECONDITION_FAIL', ...drive }
}

// 全 PASS / 无 FAIL → 直接收口，不浪费诊断 agent。
const fails = (drive.rows || []).filter((r) => r.status === 'FAIL')
if (fails.length === 0) {
  log(`✅ e2e 无 FAIL（PASS ${drive.pass} · SKIP ${drive.skip}）`)
  return { verdict: 'OK', ...drive }
}

phase('Diagnose')
const diag = await agent(
  `host-cli↔Go e2e 有 ${fails.length} 个 FAIL，读日志 \`${drive.logPath}\` 定位**每个**失败根因（只读，不改文件）。
失败项：${fails.map((f) => `${f.label}（${f.note}）`).join(' · ')}

逐个判别（grep 日志）：① WS 是否连上（"主 WS 已连接" / "RAW WS inbound hello"）② 是否收到 hello 握手（connectionId 盖章）③ 对应命令的 HTTP 是否出站 + Go 返回码 ④ Go/grpc 侧是否报错（session 过期 / grpc service fail / no service available）⑤ 该 action 的 WS 帧是否压根没来。
对每个 FAIL 给一句根因 + 一个下一步动作。`,
  { label: 'diagnose', phase: 'Diagnose', agentType: 'general-purpose' },
)

return {
  verdict: fails.length ? 'HAS_FAILURES' : 'OK',
  ...drive,
  diagnosis: diag,
}
