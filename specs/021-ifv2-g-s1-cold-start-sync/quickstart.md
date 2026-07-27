# Quickstart: G-S1 hello 全量增量

## 1. Source Preflight

```bash
cd /System/Volumes/Data/workspace/rust/helix
git status --short
git rev-parse HEAD
git -C /System/Volumes/Data/workspace/angular/cses-client rev-parse HEAD
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ rev-parse HEAD
```

确认三仓 pin、前序 `G-16c` review-green，以及本次 fresh group prerequisite。脏工作树只记录，不覆盖。

## 2. Helix

```bash
cargo check -p helix-im
cargo test -p helix-im --test gs1_full_flow_test -- --nocapture
cargo test -p helix-im --test gs1_wdio_fixture_contract_test -- --nocapture
```

期望：完整场景、Frame 字段、Golden 与 provenance 全通过；普通测试不修改 `crates/helix-im/tests/fixtures/gs1-wdio/`。

## 3. PC

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:gs1:mock
yarn message:wdio:gs1:single-real
yarn message:wdio:gs1:dual-real
```

真实 lane 必须先 seed，再完全停止应用并执行：

```bash
npm run message:v4:reset-offline-storage
```

WDIO 隔离 profile 由 runner 将 actor/observer profile root 显式传给同一 reset CLI。期望：只删除 444/678 或显式 profile 下的 Helix SQLite main/WAL/SHM/journal，零残留后复用同路径 restart/cold sync；Mock 与 Real 分层报告，PC Actor/Observer 对 isolated Gate dataset 的远端/本地对账均为 `missing=0、extra=0、mismatch=0`。无法启动真实环境时标记 Blocked，不用 Mock 替代。

## 4. Mobile

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='GS1FrameContractTest.*'
./scripts/run-uc01-16-gtest.sh
```

期望：userA/userB 先 seed，production runtime 完全 dispose 后仅删除各自同一路径 Helix SQLite main/WAL/SHM/journal，再 restart/cold sync；两 persona 对 isolated Gate dataset 的远端/本地对账均为 `missing=0、extra=0、mismatch=0`。mirror parity、生产 SDK/Bridge/SurfaceRuntime/QuickJS typed view、V2-only hard-cut 全通过。仅当本 Gate 修改 Venus/QuickJS 接线能力时追加专属 Venus Gate，否则使用共享回归。

## 5. Review

汇总 Helix、Fixture、PC Mock、PC Real、Mobile Contract、四 persona 物理删库/同路径冷启动/远端本地全量对账、production-boundary 与行为/代码/Frame V2 Review。required pass=100%、fail=0、skip=0 且所有 parity 差异为 0 才可 review-green。
