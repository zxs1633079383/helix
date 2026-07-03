export const meta = {
  name: 'host-cli-go-e2e',
  description: 'legacy workflow name: run canonical scripts/host-cli-FULL-uc-e2e.sh + structured PASS/FAIL diagnosis',
  whenToUse: 'cses-im-server(:8066) up 后，验证 host-cli ↔ Go 全量 UC 行为。args 可传 cookieId/companyId/waitTo 覆盖配置。',
  phases: [
    { title: 'Drive', detail: '跑 host-cli-FULL-uc-e2e.sh，解析全量 UC 结果表' },
    { title: 'Diagnose', detail: '仅失败时：读日志定位根因（未连/未握手/Go 报错/命令未达）' },
  ],
}

// args: { cookieId?, companyId?, waitTo? }
// ⚠️ args 以 JSON 字符串注入（README §约束 2026-06-12 踩坑）→ 归一化 shim 必抄。
const a = typeof args === 'string' ? JSON.parse(args || '{}') : args || {}

const envPairs = [
  ['HELIX_COOKIE_ID', a.cookieId],
  ['HELIX_COMPANY_ID', a.companyId],
  ['WAIT_TO', a.waitTo],
].filter(([, v]) => v != null && v !== '')
const envPrefix = envPairs.map(([k, v]) => `${k}='${String(v).replace(/'/g, `'\\''`)}'`).join(' ')

const RESULT_SCHEMA = {
  type: 'object',
  required: ['exitCode', 'pass', 'fail', 'skip', 'rows', 'logPath'],
  properties: {
    exitCode: { type: 'integer' },
    pass: { type: 'integer' },
    fail: { type: 'integer' },
    skip: { type: 'integer' },
    logPath: { type: 'string', description: '脚本输出的日志绝对路径' },
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
  `在仓库根跑 host-cli↔Go 全量 UC e2e 驱动脚本，**只运行、不改任何文件**：

\`\`\`bash
${envPrefix ? `${envPrefix} ` : ''}bash scripts/host-cli-FULL-uc-e2e.sh
\`\`\`

脚本会前置检查 Go 可达性，逐 UC 真连 Go，最后打印 PASS/FAIL/SKIP 表。
解析它的 stdout：① 退出码 ② 表格每行（PASS/FAIL/SKIP + label + note）③ 汇总行 ④ 若有日志路径也返回。
按 schema 返回结构化结果。**禁止**自己改脚本/源码/补跑别的命令——只忠实运行 + 解析。`,
  { label: 'drive:e2e', phase: 'Drive', schema: RESULT_SCHEMA, agentType: 'general-purpose' },
)

if (!drive) return { error: 'drive agent 失败（脚本未跑或解析失败）' }
if (drive.exitCode === 2) {
  log('⛔ 前置不满足（Go 不可达 / 缺身份）——未跑任何 action')
  return { verdict: 'PRECONDITION_FAIL', ...drive }
}

const fails = (drive.rows || []).filter((r) => r.status === 'FAIL')
if (fails.length === 0) {
  log(`✅ e2e 无 FAIL（PASS ${drive.pass} · SKIP ${drive.skip}）`)
  return { verdict: 'OK', ...drive }
}

phase('Diagnose')
const diag = await agent(
  `host-cli↔Go e2e 有 ${fails.length} 个 FAIL，读日志 \`${drive.logPath || 'stdout'}\` 定位**每个**失败根因（只读，不改文件）。
失败项：${fails.map((f) => `${f.label}（${f.note}）`).join(' · ')}

逐个判别：① WS 是否连上 ② 是否收到 hello 握手 ③ 对应命令的 HTTP 是否出站 + Go 返回码 ④ Go/gRPC 侧是否报错 ⑤ 该 action 的 WS 帧是否压根没来。
对每个 FAIL 给一句根因 + 一个下一步动作。`,
  { label: 'diagnose', phase: 'Diagnose', agentType: 'general-purpose' },
)

return {
  verdict: fails.length ? 'HAS_FAILURES' : 'OK',
  ...drive,
  diagnosis: diag,
}
