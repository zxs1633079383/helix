# Quickstart: G-09 取消定时

## 1. Source Preflight

```bash
cd /System/Volumes/Data/workspace/rust/helix
git status --short
git rev-parse HEAD
git -C /System/Volumes/Data/workspace/angular/cses-client rev-parse HEAD
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ rev-parse HEAD
```

确认三仓 pin、前序 `G-08` review-green，以及本次 fresh group prerequisite。脏工作树只记录，不覆盖。

## 2. Helix

```bash
cargo check -p helix-im
cargo test -p helix-im --test g09_full_flow_test -- --nocapture
cargo test -p helix-im --test g09_wdio_fixture_contract_test -- --nocapture
```

期望：完整场景、Frame 字段、Golden 与 provenance 全通过；普通测试不修改 `crates/helix-im/tests/fixtures/g09-wdio/`。

## 3. PC

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:g09:mock
yarn message:wdio:g09:single-real
yarn message:wdio:g09:dual-real
```

期望：Mock 与 Real 分层报告；无法启动真实环境时标记 Blocked，不用 Mock 替代。

## 4. Mobile

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='G09FrameContractTest.*'
./scripts/run-uc01-16-gtest.sh
```

期望：mirror parity、生产 SDK/Bridge/SurfaceRuntime/QuickJS typed view、V2-only hard-cut 全通过。仅当本 Gate 修改 Venus/QuickJS 接线能力时追加专属 Venus Gate，否则使用共享回归。

## 5. Review

汇总 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与行为/代码/Frame V2 Review。required pass=100%、fail=0、skip=0 才可 review-green。
