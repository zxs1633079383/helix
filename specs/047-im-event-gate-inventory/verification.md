# Verification Procedure

## Required Runs

- **V-001**: `jq '[.uc_cases[]] | {total:length, active:(map(select(.lifecycle ==
  "active")) | length), excluded:(map(select(.lifecycle == "excluded")) | length)}'
  crates/helix-im/contracts/strict-matrix.v1.json`，预期精确输出
  `{"total":50,"active":47,"excluded":3}`。
- **V-002**: 从 `spec.md` 提取本轮截图 Gate ID，与冻结的 30 Gate 清单做集合对账；
  预期缺失 0、额外 0、重复 0。
- **V-003**: 对本轮 30 个 Gate 检查 cses-client 的 `mock`、`single-real`、
  `dual-real` WDIO 源入口；预期 90 个 Gate-lane 组合均有来源。G-01 mock
  由 suite 与 sent/failed/WS-dialog 场景组成，不要求虚构一个统一
  `g01-mock.e2e.ts`。该检查只证明源码存在。
- **V-004**: `rg -n 'placeholder|\[Replace|TODO|TBD'
  specs/047-im-event-gate-inventory`，预期无未解析模板占位符。
- **V-005**: `python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance`，
  预期 mandatory `r1_spec` 从 blocked 修复为 passed，并进入等待 plan 的状态。
- **V-006**: `git diff --check`，预期无空白错误。

## Evidence Rules

本轮属于 specify 阶段，不运行 Cargo、WDIO 或真实链，也不把静态检查登记成运行态
Green。进入 plan/implement 后，命令原始输出必须保存到 `evidence/`，并通过
`loop_engine.py record-evidence` 登记；失败输出不得用总结性文字覆盖。
