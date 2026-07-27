# Quickstart: G-S2 sync notify

## 1. Source Preflight

```bash
cd /System/Volumes/Data/workspace/rust/helix
git status --short
git rev-parse HEAD
git -C /System/Volumes/Data/workspace/angular/cses-client rev-parse HEAD
git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ rev-parse HEAD
```

确认三仓 pin、前序 `G-S1` review-green，以及本次 fresh group prerequisite。脏工作树只记录，不覆盖。

## 2. Helix

```bash
cargo check -p helix-im
cargo test -p helix-im --test gs2_full_flow_test -- --nocapture
cargo test -p helix-im --test gs2_wdio_fixture_contract_test -- --nocapture
```

期望：完整场景、Frame 字段、Golden 与 provenance 全通过；普通测试不修改 `crates/helix-im/tests/fixtures/gs2-wdio/`。

## 3. PC

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
yarn message:wdio:gs2:mock
yarn message:wdio:gs2:single-real
yarn message:wdio:gs2:dual-real
```

期望：Mock 与 Real 分层报告；无法启动真实环境时标记 Blocked，不用 Mock 替代。

## 4. Mobile

```bash
cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
./scripts/run-gtest-js-spec.sh surface-runtime --gtest-filter='GS2FrameContractTest.*'
./scripts/run-uc01-16-gtest.sh
```

期望：mirror parity、生产 SDK/Bridge/SurfaceRuntime/QuickJS typed view、V2-only hard-cut 全通过。仅当本 Gate 修改 Venus/QuickJS 接线能力时追加专属 Venus Gate，否则使用共享回归。

## 5. Review

汇总 Helix、Fixture、PC Mock、PC Real、Mobile Contract、production-boundary 与行为/代码/Frame V2 Review。required pass=100%、fail=0、skip=0 才可 review-green。

## 6. Four-persona Cold-start Acceptance

1. PC 444/678 与 Mobile userA/userB 使用生产流程创建 fresh group、基线消息及
   离线消息，记录每个 persona 的原 Helix SQLite 路径。
2. 完全停止 PC Tauri 进程与 Mobile runtime；PC 通过
   `npm run message:v4:reset-offline-storage` 或显式 `--profile-root`，
   Mobile 通过 GTest guarded helper，仅删除 main/WAL/SHM/journal。
3. 断言 residual=0、path unchanged、`remoteDeletionPerformed=false`。
4. 同路径重启并执行 production `im_sync_channels`，等待 PersistOk 后的
   `message.left-dialog-list` FrameApplied/typed view。
5. 对四 persona 分别比较 PostgreSQL 与 SQLite 的 Gate-scoped canonical rows；
   remote/local hash 必须相等，missing/extra/mismatch 必须全部为 0。
