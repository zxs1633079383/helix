# Verification: InteractionFrame V2 双仓硬切

## Phase 1 Source Lock

**Captured**: `2026-07-23T03:52:13-07:00`

| Repository | Source branch | Source HEAD | Source worktree state | Isolated branch | Isolated path |
|---|---|---|---|---|---|
| Helix | `main` | `3b430c6cb1cd4af615d487dde2c9788b7cfda257` | dirty: `SESSION.md`、WS parser/tests、G01 V2 流程图、loop ledger | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/rust/helix/worktrees/018-interaction-frame-v2-cutover` |
| cses-client | `feat/helix-reactor-im` | `8d71085453c62b966d82b6c947f68a20cc638e3f` | dirty: message-v4 runtime view/content/SCSS 六项 | `codex/018-interaction-frame-v2-cutover` | `/System/Volumes/Data/workspace/angular/cses-client/worktrees/018-interaction-frame-v2-cutover` |
| cses-im-server | `main` | `6ddb5245364128732fea1c1ea48977001394a76f` | untracked `=`；本 Feature 默认只读 | 不创建 | `/System/Volumes/Data/workspace/golang/cses-im-server` |

两个隔离 worktree 创建后均为 clean；源工作树 branch/HEAD 前后未变化。所有实现、测试和提交只在隔离分支发生，除非最终 pin/验收步骤明确要求回写。

## Frozen Inputs

| Input | Value |
|---|---|
| cses-client `helix-core` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-im` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-host` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| cses-client `helix-driver-native` pin | `154c7ba606629c402aa039f9c2c1e28cd0e2859c` |
| source `src-tauri/Cargo.lock` SHA-256 | `a789269b5fc2edd1a09c6bb3114b60a2658b7e76fb097017a4c7fea6a43ed6f9` |
| `strict-matrix.v1.json` SHA-256 | `d3461beb760255f6ae733e3b5f7ceccd32dca22faf8adf0c76c29e0d6786b345` |
| InteractionFrame fixture manifest SHA-256 | `71c281e5879ecf2d61f71363777f7b2b7915170403d33a0faff8964fcf81b1da` |

`Cargo.lock` 是 cses-client 源工作树的本地 ignored artifact，隔离 worktree 初始不存在；首次 Native 构建生成的新 lock 必须作为候选证据重新 hash，不能把上述 source hash 当作最终 pin。

## Phase Status

| Task | Status | Evidence |
|---|---|---|
| T001 | PASS | 三仓 branch/HEAD/dirty、四包 pin、lock/matrix/fixture hash 已记录 |
| T002 | PASS | [discovery.md](discovery.md) 已按当前锁定源码更新 V1/合法独立 v1 inventory |
| T003 | PASS | `python3 scripts/verify-interaction-frame-v2-index.py` 返回 19/19/19、25 Gate、T021-T077 完整 |
| T004 | PASS | [quickstart.md](quickstart.md) 固定当前可执行 focused、样式构建、Native、双库只读入口与 artifact root |
| T005 | PASS | Helix/cses-client 隔离 worktree 已创建且源 ref 前后不变 |

## Required Runs

后续 V-001 至 V-008 的原始输出必须写入 `evidence/`，并通过 `loop_engine.py record-evidence` 登记 hash。失败输出不得被摘要文本覆盖。

## Phase 2 Shared V2 Foundation

**Completed subset captured**: `2026-07-23T05:35:00-07:00`

| Task | Status | Evidence |
|---|---|---|
| T006 | PASS | Helix V2 contract + V1 fail-closed tests；`interaction_frame_contract_test` 17/17 PASS |
| T007 | PASS | client V2-only codec + V1/unknown rejection；focused Jest PASS |
| T008 | PASS | epoch/stale/atomic/directive/ack runtime tests；focused Jest PASS |
| T009 | PENDING | 25-Gate profile/counter/artifact-isolation evidence contract 尚未实现 |
| T010 | PASS | Helix production writer 唯一输出 InteractionFrame V2；commit `ea415a818deaf38c874232db4024df2e65584d96` |
| T011 | PASS | client production codec 仅接受 schemaVersion=2，无 V1 normalize/union |
| T012 | PASS | client slot state 纳入 surfaceEpoch，epoch/revision 整帧 staged swap |
| T013 | PASS | Angular → Tauri → Helix private `FrameApplied` typed command，content-free ack tests PASS |
| T014 | PASS | Helix matrix epoch 11、`InteractionFrame/v2`、fixture producer pin 与 client mirror/hash 对齐 |
| T015-T020 | PENDING | Native profile、counter/reducer、SCSS/CSS、manifest、diagnostics 与 shared validators 尚未完成 |

### Formal Revisions And Hashes

| Artifact | Value |
|---|---|
| Helix V2 producer commit | `ea415a818deaf38c874232db4024df2e65584d96` |
| Helix formal pin (matrix + fixture manifest) | `d899a6033c90a153b5b009a62bee0e3a17dd9ad8` |
| cses-client V2 consumer commit | `11bd41b1b3a540ee26d6a95ab4b0468d929d98f5` |
| client generated `Cargo.lock` SHA-256 | `c6aceb0b8b5e6c9f91d2ca17f47b240417c0a5dab97d00925ec0c32b1f119f95` |
| client fixture manifest SHA-256 | `572914e80235227a690ffefeafcc2e5280f5e55e6807b26e3a824f6f010c4447` |
| client generated Strict Matrix TS SHA-256 | `f49fe63be8005d1bab7446f7a8315e6272cb0d703f57cec98fe62a67d5318582` |

### Verification Runs

| Command | Result | Wall Time / Notes |
|---|---|---|
| `cargo test -p helix-im --quiet` | PASS | 全量 lib + integration；约 26s；ignored tests 保持原状态 |
| focused Helix V2 contract/manifest/projection tests | PASS | 68/68；1.7s |
| client codec/runtime/action/parity/store/shell focused Jest | PASS | 7 suites、71 tests；13.1s |
| client runtime/shell post-ack focused Jest | PASS | 2 suites、33 tests；17.0s |
| `npx tsc -p src/tsconfig.app.json --noEmit` | PASS | isolated worktree 复用 ignored `node_modules` 与 `release.local.json` |
| `cargo check --manifest-path src-tauri/Cargo.toml` | PASS | 首轮因 isolated worktree 缺 ignored staged resources/dist 失败；补齐同源 ignored artifacts 后通过；既有 warnings |
| focused Native `frame_applied_queues_the_content_free_private_command` | PASS | 1/1 |
| `npm run test:message-v4:strict-contract` | PASS | epoch 11；Helix pin/hash 与 generated consumer 一致；Cargo.lock matched |

Phase 2 尚未完成，`SharedV2Ready` 不成立；不得据此进入 25 Gate READY 或宣称 Native/PG/SQLite/DOM 全链路通过。
