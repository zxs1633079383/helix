# Helix Harness — 触发操作手册

> 机械检查交给脚本，判断工作按需触发；本页只保留当前可执行入口。

## 触发矩阵

| 触发事件 | 执行动作 | 方式 |
|---|---|---|
| 保存 Rust 文件 | `cargo fmt --all --quiet` | Codex PostToolUse hook |
| 新会话 | 由 `scripts/session-start-context.sh` 读取 `docs/ROADMAP.md` P0、本手册与前端 UC 投影真源 | Codex SessionStart hook |
| 准备提交或 push | `scripts/gate.sh` | 人工；pre-push hook 兜底 |
| 结构检查 | `scripts/structure-gate.sh` | gate 第 11 步 |
| 架构 review 检查点 | `scripts/review-gap.sh`；红灯时按 AGENTS.md §4.7 独立 review 并更新 ledger | commit 检查点 |
| 分支身份核对 | `scripts/branch-identity.sh --session` | 开局、worktree 收尾或身份不清时 |
| FFI 仓内验收 | `scripts/ffi-verify.sh --no-demo` | gate 第 9 步 |
| 真 Go 全 UC | `scripts/host-cli-FULL-uc-e2e.sh` | 环境就绪后人工触发 |
| 新业务模块 | `/helix-new-module` | 按需 skill |
| 写回放测试 | `/helix-replay-test` | 按需 skill |
| 承压验收 | `/helix-stress-verify` | 按需 skill |
| agent 路由 | `.agents/scripts/agent-route --summary "..." --paths <path>` | 需要拆分任务时 |
| 同根因 ≥3 / 用户拍板 / Spec | 新建 `docs/harness/HX-C{NNN}` 并同步两处索引 | 人工 |

## 自动与人工边界

- 自动：fmt、开局/收尾提醒、pre-push gate 等机械检查。
- 人工：架构判断、跨仓决策、发布、回滚和多 agent 拆分。
- 后台 agent 完成依赖任务通知，不做固定周期空轮询；外部 CI、部署队列等不可追踪状态才使用监控机制。

## 索引

- 项目规则：[`AGENTS.md`](../../AGENTS.md)
- Harness 索引：[`README.md`](./README.md)
- Agent 路由：[`.agents/README.md`](../../.agents/README.md)
- Review 台账：[`docs/review/workflow-review-ledger.md`](../review/workflow-review-ledger.md)
