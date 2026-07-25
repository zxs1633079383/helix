# Helix Gate Frame Producer Boundary

## Responsible Agent

- `.agents/agents/domains/helix-gate-frame-producer-domain-agent.md`

## Rules

- 只拥有 `helix-im` 逐 Gate sans-IO 回放、字段断言和 canonical Frame 生产语义。
- Frame 必须经 `ImModule::handle -> Effect::Emit` 取得，不得手写业务结果。
- Persist barrier、独立 correlation 和确定性输出必须由测试明确证明。
- 不声明 Tauri、SQLite 执行器、Go、WebSocket 或 UI 已通过。
