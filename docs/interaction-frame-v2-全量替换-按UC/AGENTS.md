# InteractionFrame V2 逐 Gate 路由

本目录按 Gate 管理 Helix、PC 与 Mobile 的 V2 producer、consumer 和真实链验收合同。

## Responsible Agents

- 总控：`.agents/agents/project/gate-acceptance-orchestrator-agent.md`
- Helix producer：`.agents/agents/domains/helix-gate-frame-producer-domain-agent.md`
- Fixture 供应链：`.agents/agents/domains/frame-fixture-supply-chain-domain-agent.md`
- 独立裁决：`.agents/agents/reviewers/cross-platform-gate-reviewer-agent.md`

## Rules

- 每个 Gate 必须区分 producer、consumer contract、产品实现和真实链证据等级。
- Helix canonical Fixture 只能来自 `ImModule::handle` 的 `Effect::Emit`。
- PC 与 Mobile 只读同步并机械 apply Frame，不得补算 Helix 业务字段。
- Mock、GTest 或 Fixture replay 不能替代真实 Tauri、Go、WS、DOM 或真机证据。
- Gate 文档必须区分已验证事实、目标状态、阻塞项和最大可声明结论。
- 全部 Gate 完成前保留零引用历史 V1 资产；已迁移 Gate 的生产可达路径必须 V2-only。

## Handoff

Gate producer Green 后才能同步 Fixture；三端 required lane 闭合后，再交由跨端 Reviewer
依据同一 source pin、inventory 和业务语义裁决。
