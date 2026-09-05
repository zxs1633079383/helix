# Canonical Fixture 路由（历史）

> `RETIRED_HISTORICAL`：当前 MessageV3 Gate producer 位于
> [`../message_v3/README.md`](../message_v3/README.md)。本目录仅维护 `real_go_frames/` 真帧
> 样本，不再承载 Gate 分母或 Frame producer。

本目录仅维护历史 producer artifact 与 `real_go_frames/` 真帧样本。

## Required Reference

- 历史 Gate 目录、证据边界与真帧更新入口：
  [`README.md`](README.md)

## Responsible Agents

- 历史 Producer：`.agents/agents/domains/helix-gate-frame-producer-domain-agent.md`
- Supply chain：`.agents/agents/domains/frame-fixture-supply-chain-domain-agent.md`
- Reviewer：`.agents/agents/reviewers/cross-platform-gate-reviewer-agent.md`

## Rules

- 历史三端 Fixture 供应链执行
  [`../../../../docs/三端功能测试基线/AI执行合同.md`](../../../../docs/三端功能测试基线/AI执行合同.md)。
- 禁止手工编辑 Frame JSON 来修复 PC/Mobile 测试。
- 历史 Gate Fixture 才要求显式 inventory 与 `provenance.json`；当前 Gate 使用
  `tests/message_v3` 的四份 generated JSON。
- provenance 至少记录 fixtureId、文件名、schema、producer commit、raw/semantic hash、
  surfaces 和参数化白名单。
- consumer mirror 不是真源；任何业务字段变化必须回到 ScenarioRunner。
- `real_go_frames/` 真帧新增或变更时同步更新其 README、parser/replay 测试和来源说明；不
  修改 Spec 047 的 Gate 分母。
- 旧 Gate/Fixture 目录已删除，不按历史表恢复。
- 一个 Gate 可以混合不同 Surface schema；合同版本以 Frame 与 provenance 为准，消费者不得把
  `.frame-v2.json` 或 `.frame-v3.json` 写死为通用规则。
