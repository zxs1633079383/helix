# Helix Gate 测试路由

本目录的逐 Gate ScenarioRunner、full-flow、Fixture producer 和字段断言由
`.agents/agents/domains/helix-gate-frame-producer-domain-agent.md` 负责。

涉及跨仓 Fixture inventory、provenance、hash 或参数白名单时，同时路由
`.agents/agents/domains/frame-fixture-supply-chain-domain-agent.md`。

## 不变量

- Frame 只能来自公开 `ImModule::handle` 的 `Effect::Emit`，不得由测试手写成业务结果。
- full-flow 只证明 Helix sans-IO 边界，不声明 Tauri、SQLite 执行器、Go、WS 或 UI 已通过。
- 每个 Gate 的 ScenarioRunner 必须表达真实 correlation 和 Persist barrier。
- 新增或修改测试/helper 方法必须有紧邻精简注释。
- 普通测试只比较 Golden；只有 Gate 明确规定的 update 环境变量可以覆盖 Fixture。

## Handoff

producer Green 后，将 Fixture 和 provenance 交给 supply-chain Agent；消费者验证完成后，
由 `cross-platform-gate-reviewer-agent` 做独立裁决。
