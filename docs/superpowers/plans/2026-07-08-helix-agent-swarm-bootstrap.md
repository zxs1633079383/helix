# Helix Agent Swarm Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 为 Helix 初始化 repo-local `.agents` 蜂群，让 Router 能按模块、领域、约束和证据边界派发任务。

**Architecture:** `.agents/` 只保存编排事实、agent 协议、manifest、Mermaid 图谱和确定性脚本，不进入 Rust runtime。Helix 既有 crate 级 `AGENTS.md`/`CLAUDE.md` 镜像不由 bootstrap 覆盖；目录边界继续由现有项目指令承担。

**Tech Stack:** Markdown, JSON manifests, Mermaid `.mmd`, Python stdlib `agentctl.py`, Helix docs/harness/script gates.

## Global Constraints

- 默认中文沟通、报告、review findings、终态行；命令、路径、代码符号可保留英文。
- 只初始化 `.agents/` 和本计划/SESSION 记录，不修改 Rust 源码。
- `helix-core` 的 sans-IO / runtime-agnostic 不变量不得被 agent 编排层污染。
- 不覆盖现有 crate 级 `AGENTS.md`/`CLAUDE.md` 镜像文件。
- 验证以 `review_instance.py`、`.agents/scripts/*`、`git diff --check` 为准。

---

### Task 1: Bootstrap Swarm Skeleton

**Files:**
- Create/modify: `.agents/**`
- Modify: `.gitignore`

**Interfaces:**
- Consumes: `/Users/xpa-12/.agents/skills/agent-swarm-bootstrap/scripts/bootstrap.py`
- Produces: `.agents/bin/agentctl.py`, `.agents/scripts/agent-*`, `.agents/manifests/*.json`

- [ ] Run bootstrap with explicit Helix module/domain definitions and `--no-directory-agents`.
- [ ] Confirm generated module count is 8 and domain count is 9.
- [ ] Confirm no crate-level `AGENTS.md` was overwritten.

### Task 2: Replace Placeholders With Helix Facts

**Files:**
- Modify: `.agents/README.md`
- Modify: `.agents/router/project-router.md`
- Modify: `.agents/router/routes.json`
- Modify: `.agents/manifests/*.json`
- Modify: `.agents/templates/*.md`
- Modify: `.agents/agents/**/*.md`

**Interfaces:**
- Consumes: `docs/GOAL.md`, `docs/ROADMAP.md`, `docs/harness/OPERATING.md`
- Produces: routeable Helix module/domain/reviewer protocols

- [ ] Add authoritative references and active HX-C / P0 constraint subscriptions.
- [ ] Map modules to domains and reviewer escalation rules.
- [ ] Translate prompt templates to Chinese and include the required language/readability block.

### Task 3: Add Mermaid Module And Domain Graphs

**Files:**
- Create: `.agents/diagrams/project-domain-map.mmd`
- Create: `.agents/diagrams/modules/*.mmd`
- Create: `.agents/diagrams/domains/*.mmd`
- Create: `.agents/diagrams/README.md`

**Interfaces:**
- Consumes: `.agents/manifests/modules.json`, `.agents/manifests/domains.json`
- Produces: reusable visual context for every module and every domain

- [ ] Create one project-level architecture graph.
- [ ] Create one graph per module showing owned paths, domains, hard constraints, and reviewers.
- [ ] Create one graph per domain showing source paths, participating modules, and validation gates.

### Task 4: Validate Instance

**Files:**
- Read-only validation except final SESSION `DONE` line if all checks complete.

**Interfaces:**
- Consumes: `.agents/scripts/*`, bootstrap review script
- Produces: validation evidence

- [ ] Run `python3 /Users/xpa-12/.agents/skills/agent-swarm-bootstrap/scripts/review_instance.py --repo "$PWD"`.
- [ ] Run `.agents/scripts/agent-check-constraints`.
- [ ] Run `.agents/scripts/agent-route --summary "send message sync projection" --paths crates/helix-im/src/send`.
- [ ] Run `.agents/scripts/agent-check-report --sample`.
- [ ] Run `.agents/scripts/agent-clean-runs --dry-run`.
- [ ] Run `git diff --check`.
