# Canonical Frame Fixture 路由

本目录只保存 Helix producer 生成的 canonical artifact。

## Responsible Agents

- Producer：`.agents/agents/domains/helix-gate-frame-producer-domain-agent.md`
- Supply chain：`.agents/agents/domains/frame-fixture-supply-chain-domain-agent.md`
- Reviewer：`.agents/agents/reviewers/cross-platform-gate-reviewer-agent.md`

## Rules

- 禁止手工编辑 Frame JSON 来修复 PC/Mobile 测试。
- 每个 Gate 必须有显式 inventory 与 `provenance.json`。
- provenance 至少记录 fixtureId、文件名、schema、producer commit、raw/semantic hash、
  surfaces 和参数化白名单。
- consumer mirror 不是真源；任何业务字段变化必须回到 ScenarioRunner。
- 新增 Fixture 时同步更新两个 consumer 的 loader、parity 测试和机器分母。
