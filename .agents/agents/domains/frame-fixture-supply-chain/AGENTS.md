# Frame Fixture Supply Chain Boundary

## Responsible Agent

- `.agents/agents/domains/frame-fixture-supply-chain-domain-agent.md`

## Rules

- 只负责 canonical Fixture 的 inventory、provenance、hash、参数白名单和只读分发。
- 不修改 Helix Frame 业务内容，不为 PC 或 Mobile 补充业务默认值。
- raw 与 semantic hash 必须分别验证，不能用其中一个掩盖另一个漂移。
- consumer mirror 不是真源，source pin 不一致时停止跨仓 Green 声明。
