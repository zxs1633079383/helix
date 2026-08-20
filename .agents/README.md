# Helix Agent Swarm

`.agents/` 是 Helix 的 repo-local 蜂群协议包：它把本仓的模块边界、DDD/领域边界、reviewer 角色、权威引用、约束订阅和 Mermaid 图谱固化成可路由资产。

## 使用方式

- 任务入口先走 `.agents/scripts/agent-route --summary "..." --paths <path>`，拿到 primary agents、reviewer agents 和 required references。
- worker prompt 使用 `.agents/templates/agent-task.md` 或 `.agents/templates/worktree-agent-task.md` 渲染。
- reviewer prompt 使用 `.agents/templates/reviewer-task.md`，reviewer 必须把无证据 claim 标成 `unverified`。
- agent 报告必须满足 `.agents/schemas/agent-report.schema.json`：`agent/status/changed_files/evidence/unverified/risks`。
- `.agents/runs/` 与 `.agents/tmp/` 是运行时目录，不进 git。

## Repo-local Skills

- `$helix-command-driver-host-test`（`.agents/skills/helix-command-driver-host-test/SKILL.md`）：为新增或修改 Command 接入 `helix-driver-host`/Native real-chain 测试，按 TDD、共享 adapter、证据分层和 run-owned 清理执行。
- `$messagev3-cross-layer-emit-diagnosis`（`.agents/skills/messagev3-cross-layer-emit-diagnosis/SKILL.md`）：沿 Angular → Ipc → Helix → Go HTTP/WS → Persist/Projection → EventSink → Angular 逐跳定位事件或字段丢失，并指导 TDD 修复与证据分层。
- `$messagev3-cross-layer-coding`（`.agents/skills/messagev3-cross-layer-coding/SKILL.md`）：按契约冻结、TDD 红绿、owner 分层、绝对 projection 和真实链证据实施跨 Go/Helix/driver/Angular 的 MessageV3 功能。

## Helix 特殊边界

- `helix-core` 永远只接受运行时无关、sans-IO、业务无关的任务。
- `helix-im` 拥有 IM 业务真相；driver/native/ffi/host-cli 只做适配和验证。
- `helix-driver-web` 是正式 Web 平台壳：Rust 持有业务 HTTP/query、sync/cursor/echo 与 WS 帧解析；`browser/` glue 只接 Browser API 和物理 WebSocket，不包含产品 UI 或业务语义。
- crate 级 `AGENTS.md` 是源码目录的就近指令文件。
- `.agents/agents/modules/<module>/AGENTS.md` 与 `.agents/agents/domains/<domain>/AGENTS.md` 是蜂群内部的模块/领域边界文件。

## Gate 合同与验收

- `project/gate-contract-specification-agent` 把自然语言编译成经过源码求证的 Gate YAML、
  初始 Strict Matrix requirements 和 Spec Kit 输入；它不实现业务或回填 PASS。
- `project/gate-acceptance-orchestrator-agent` 只在合同 source-validated 后编排三仓实施。
- `domains/helix-gate-frame-producer-domain-agent` 与
  `domains/frame-fixture-supply-chain-domain-agent` 分别拥有 producer 和只读分发。
- `reviewers/cross-platform-gate-reviewer-agent` 保持独立，只裁决实际 evidence 交集。
- 合同模板、schema 和明确标注 EXAMPLE 的样例位于 `.agents/{templates,schemas,examples}/`。

## 图谱

- 总图：`.agents/diagrams/project-domain-map.mmd`
- 模块图：`.agents/diagrams/modules/*.mmd`
- 领域图：`.agents/diagrams/domains/*.mmd`

## 模块与领域 AGENTS

- 模块边界：`.agents/agents/modules/*/AGENTS.md`
- 领域边界：`.agents/agents/domains/*/AGENTS.md`
- 角色协议仍保留在 `.agents/agents/modules/*-agent.md` 与 `.agents/agents/domains/*-domain-agent.md`
