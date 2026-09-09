# Helix Harness — 触发操作手册

> 机械检查交给脚本，判断工作按需触发；本页只保留当前可执行入口。

## 触发矩阵

| 触发事件 | 执行动作 | 方式 |
|---|---|---|
| 保存 Rust 文件 | `cargo fmt --all --quiet` | Codex PostToolUse hook |
| 新会话 | 由 `scripts/session-start-context.sh` 读取 `docs/ROADMAP.md` P0、本手册与前端 UC 投影真源 | Codex SessionStart hook |
| 按需本地检查 | `scripts/gate-fast.sh` | 人工按改动范围执行，不安装 pre-push hook |
| 合并前 / GitLab CI | `node scripts/messagev3/phase1-5-real-chain.mjs --all` | 83 个可执行 MessageV3 real-chain Gate；端点默认 `pre`，证据不完整即非零退出 |
| CI 诊断（`MESSAGEV3_DIAGNOSE=1`） | `node scripts/messagev3/ci-diagnose.mjs` 后执行 `--gate MV3-G07B --env pre` | 前置失败即停止；此模式排除完整 real-chain 作业 |
| 分支身份核对 | `scripts/branch-identity.sh --session` | 开局、worktree 收尾或身份不清时 |
| FFI 仓内验收 | `scripts/ffi-verify.sh --no-demo` | gate 第 9 步 |
| 真 Go 全 UC | `scripts/host-cli-FULL-uc-e2e.sh` | 环境就绪后人工触发 |
| 新业务模块 | `/helix-new-module` | 按需 skill |
| 写回放测试 | `/helix-replay-test` | 按需 skill |
| 承压验收 | `/helix-stress-verify` | 按需 skill |
| agent 路由 | `.agents/scripts/agent-route --summary "..." --paths <path>` | 需要拆分任务时 |
| 同根因 ≥3 / 用户拍板 / Spec | 新建 `docs/harness/HX-C{NNN}` 并同步两处索引 | 人工 |

## 自动与人工边界

- 自动：fmt、开局/收尾提醒与 GitLab CI 检查；本地 push 不挂 pre-push gate。
- 人工：架构判断、跨仓决策、发布、回滚和多 agent 拆分。
- 后台 agent 完成依赖任务通知，不做固定周期空轮询；外部 CI、部署队列等不可追踪状态才使用监控机制。

GitLab CI 默认执行完整 MessageV3 real-chain；诊断变量为 `1` 时仅执行限时前置探针与 G07B。
两个 job 均声明 6 分钟 timeout、脚本预算 5 分钟；详见 [CI 诊断与限时](../../scripts/README.md#messagev3-ci-诊断与限时)。`scripts/gate.sh` 仍保留给本地/pre-push 或人工完整 Rust/FFI
检查，不再与 real-chain 争用 CI 主入口；客户端 92 Gate 合同与 real-chain 的 83 个可执行分母也不互相替代。

## 索引

- 项目规则：[`AGENTS.md`](../../AGENTS.md)
- Harness 索引：[`README.md`](./README.md)
- Agent 路由：[`.agents/README.md`](../../.agents/README.md)
