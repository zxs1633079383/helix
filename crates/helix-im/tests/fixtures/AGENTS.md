# Canonical Frame Fixture 路由

本目录只保存 Helix producer 生成的 canonical artifact。

## Required Reference

- Gate 功能、目录清单、证据边界与 producer 更新入口：
  [`README.md`](README.md)

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
- 新增、删除、拆分或重命名 Gate/Fixture 目录时，必须同步更新 `README.md` 的业务 Gate
  表、辅助 Fixture 表和 producer 入口；目录清单与文档不一致时任务不得标记完成。
- `.frame-v2.json` 仅是现有文件命名；合同版本必须读取 `schemaVersion`，不得根据文件名
  推断 Timeline 或其他 Surface 的 schema。
