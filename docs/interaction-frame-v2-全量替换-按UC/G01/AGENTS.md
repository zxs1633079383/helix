# G01 Agent Capsule

## Gate

`G01`：创建测试专用群聊后发送普通文本消息。

## Required Routing

- `project/gate-acceptance-orchestrator-agent`
- `domains/helix-gate-frame-producer-domain-agent`
- `domains/frame-fixture-supply-chain-domain-agent`
- PC：`domains/pc-gate-domain-agent`
- PC real：`domains/wdio-real-chain-domain-agent`
- Mobile：`domains/mobile-gate-contract-domain-agent`
- Mobile runtime：`domains/mobile-v2-runtime-domain-agent`
- Reviewer：`reviewers/cross-platform-gate-reviewer-agent`

## Canonical Inputs

- `README.md`
- `13-G01-Helix-PC-Mobile-V2验收合同.md`
- `handoff.md`
- `crates/helix-im/tests/g01_full_flow_test.rs`
- `crates/helix-im/tests/g01_wdio_fixture_contract_test.rs`
- `crates/helix-im/tests/fixtures/g01-wdio/provenance.json`

## Stop Conditions

- Helix actual Frame 与 Golden/provenance 不一致时，停止 consumer 同步。
- PC 或 Mobile 修改 canonical 业务字段时，退回 Fixture supply chain。
- Mock/GTest 通过但产品真实逻辑未接通，只能报告 consumer contract Green。
- 最终状态必须由跨端 reviewer 裁决，G01 Agent 不自我宣布 canonical Green。
