# Verification Procedure: InteractionFrame V2 26 Gate 总控

## Specification Runs

- **V-001**: `node --check scripts/generate_ifv2_three_repo_specs.mjs`；期望退出码 0。
- **V-002**: `node scripts/generate_ifv2_three_repo_specs.mjs`；期望 activeGates=26、childSpecs=25、gapGates=20，重复运行不改变语义内容。
- **V-003**: `python3 scripts/verify-interaction-frame-v2-index.py`；期望文档库存、机器 Gate 与全部本地链接通过。
- **V-004**: 对总控和 25 个子 Feature 执行 mandatory `loop_engine.py advance`；期望 26/26 `r1_spec.status=pass` 且 next_action 为 `run_speckit_plan`。
- **V-005**: `git diff --check`；期望无 whitespace error。

## Delivery Runs

- **V-101**: 按机器 manifest 顺序逐 Gate 执行各自 verification.md；前序未 `review-green` 时后序不得运行。
- **V-102**: 汇总 26 Gate 的 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与 Review；期望 required pass=100%、fail=0、skip=0。
- **V-103**: 26 Gate 全绿后生成 legacy/V1 删除候选清单；用户未选择前不得执行删除。
- **V-104**: 用户选择删除后重新运行三仓全量验收；期望新候选仍为 26/26 Green，删除前报告不可复用。

## Evidence Rules

- 规格检查只证明 Spec 可进入 plan，不证明任何业务 Gate 已实现或 Green。
- 每个 delivery run 必须引用原始 artifact、source pin、fixture hash 和结构化 report；聚合结论不得遮蔽 blocked/partial。
