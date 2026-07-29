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
- **V-007**: `node specs/047-im-event-gate-inventory/scripts/generate-v1-evidence.mjs`，
  预期 `.v1` 生成 30 个 Gate 目录、120 个声明合同文件、30 个捕获状态文件和
  60 个结构样例文件及 30 个 `captures/` 槽位目录；连续生成两次的全树 SHA-256 相同。
- **V-008**: `git check-ignore -v
  specs/047-im-event-gate-inventory/.v1/manifest.json` 必须命中仓库
  `**/.v1/` 规则；当前 manifest 必须如实报告 90 个原始捕获槽位中
  0 个存在、90 个缺失、总判定 `INCOMPLETE`。
- **V-009**: `.v1` 的三类捕获必须精确为 `inbound/effects/emit`；`outbound-contract.json`
  中每个 channel 都必须声明 `core_output=Effect::Emit` 和源码位置。
- **V-010**: `rg -ni 'surface|interactionframe|im:interaction:frame|\bdom\b'
  specs/047-im-event-gate-inventory/.v1`，预期无匹配；UI 层不得进入本地 v1 业务出站合同。
  `ws/frame.rs` 属于真实 transport parser，不在该禁用层范围。
- **V-011**: 30 个 Gate 必须各有 `inbound-sample.json` 和 `outbound-sample.json`；
  共 60 份样例均须为 `SOURCE_DERIVED_SAMPLE`、`runtime_capture=false`。
- **V-012**: 每个 outbound sample 的 `effect_variant` 必须为 `Effect::Emit`，
  `domain_event_bytes_json.event` 必须能在对应 `source_refs` 的生产工厂中找到，
  Native adapter view 必须满足 `channel == payload.event`。

## Evidence Rules

本轮属于 specify 阶段，不运行 Cargo、WDIO 或真实链，也不把静态检查登记成运行态
Green。进入 plan/implement 后，命令原始输出必须保存到 `evidence/`，并通过
`loop_engine.py record-evidence` 登记；失败输出不得用总结性文字覆盖。
