# Helix Agent Swarm

`.agents/` 是 Helix 的 repo-local 蜂群协议包：它把本仓的模块边界、DDD/领域边界、reviewer 角色、权威引用、约束订阅和 Mermaid 图谱固化成可路由资产。

## 使用方式

- 任务入口先走 `.agents/scripts/agent-route --summary "..." --paths <path>`，拿到 primary agents、reviewer agents 和 required references。
- worker prompt 使用 `.agents/templates/agent-task.md` 或 `.agents/templates/worktree-agent-task.md` 渲染。
- reviewer prompt 使用 `.agents/templates/reviewer-task.md`，reviewer 必须把无证据 claim 标成 `unverified`。
- agent 报告必须满足 `.agents/schemas/agent-report.schema.json`：`agent/status/changed_files/evidence/unverified/risks`。
- `.agents/runs/` 与 `.agents/tmp/` 是运行时目录，不进 git。

## Helix 特殊边界

- `helix-core` 永远只接受运行时无关、sans-IO、业务无关的任务。
- `helix-im` 拥有 IM 业务真相；driver/native/ffi/host-cli 只做适配和验证。
- `helix-driver-wasm` 是 archived/experimental；不要把它作为当前交付目标。
- crate 级 `AGENTS.md`/`CLAUDE.md` 是镜像文件，bootstrap 不写目录级 AGENTS，避免破坏现有镜像规则。

## 图谱

- 总图：`.agents/diagrams/project-domain-map.mmd`
- 模块图：`.agents/diagrams/modules/*.mmd`
- 领域图：`.agents/diagrams/domains/*.mmd`
