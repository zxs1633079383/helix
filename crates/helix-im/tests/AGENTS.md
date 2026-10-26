# Helix Gate 测试路由

本目录的逐 Gate ScenarioRunner、full-flow、canonical JSON producer 和字段断言以
[`tests/message_v3/AGENTS.md`](message_v3/AGENTS.md)、Spec 047 migration manifest
和当前 Cargo target 为真源。旧 `.agents/agents/*frame*` 只保留历史 InteractionFrame
迁移职责，不再作为 MessageV3 producer 路由。

## 不变量

- 新功能与 schema 迁移遵循当前 Spec 047 contract：Helix 测试和 canonical JSON 必须先于
  PC/Mobile consumer 测试。
- MessageV3 事件只能来自公开 `ImModule::handle` 的 `Effect::Emit { event: DomainEventBytes }`，
  不得由测试手写业务 payload。
- full-flow 只证明 Helix sans-IO 边界，不声明 Tauri、SQLite 执行器、Go、WS 或 UI 已通过。
- 每个 Gate 的 ScenarioRunner 必须表达真实 correlation 和 Persist barrier。
- 新增或修改测试/helper 方法必须有紧邻精简注释。
- 普通测试只比较 Golden；只有 Gate 明确规定的 update 环境变量可以覆盖 Fixture。

## Handoff

producer Green 后，将 Fixture 和 provenance 交给 supply-chain Agent；消费者验证完成后，
由 `cross-platform-gate-reviewer-agent` 做独立裁决。
