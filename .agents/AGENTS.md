# Agent Boundary

## Responsible Agent

- `.agents/agents/project/project-agent.md`

## Rules

- 本目录只保存 Helix repo-local 蜂群协议、manifest、模板、图谱、脚本和运行时元数据。
- 不在这里实现 Rust 业务逻辑，不替代 `docs/GOAL.md`、`docs/ROADMAP.md` 或 `docs/harness/`。
- 模块和领域必须各自有目录级边界文件：`.agents/agents/modules/<module>/AGENTS.md` 与 `.agents/agents/domains/<domain>/AGENTS.md`。
- 修改 agent 约束订阅时，同步 `.agents/manifests/constraint-subscriptions.json`、相关 agent 文档和 Mermaid 图谱。
- `.agents/runs/` 与 `.agents/tmp/` 是运行时目录，必须保持 git ignored。

## Required Constraints

<!-- agent-constraints:start -->
- `roadmap-p0-live-ledger`
- `hxc012-expect-for-substantial-task`
- `hxc015-task-notification-no-polling`
- `harness-index-invariant`
<!-- agent-constraints:end -->
