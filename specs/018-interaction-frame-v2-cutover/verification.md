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
| T009 | PASS | 25-Gate profile、v1/v2/raw/ack counter 与 candidate/evidenceEpoch 隔离测试 PASS |
| T010 | PASS | Helix production writer 唯一输出 InteractionFrame V2；commit `ea415a800b329947ce0757b8595df19b715f7d55` |
| T011 | PASS | client production codec 仅接受 schemaVersion=2，无 V1 normalize/union |
| T012 | PASS | client slot state 纳入 surfaceEpoch，epoch/revision 整帧 staged swap |
| T013 | PASS | Angular → Tauri → Helix private `FrameApplied` typed command，content-free ack tests PASS |
| T014 | PASS | Helix matrix epoch 11、`InteractionFrame/v2`、fixture producer pin 与 client mirror/hash 对齐 |
| T015 | PASS | Native `interaction-frame-v2-cutover` profile 精确选择 25 Gate，排除 G03/G03a |
| T016 | PASS | 双 actor content-free Frame probe + reducer fail-closed counters 已接通 |
| T017 | PASS | Sass 编译、style-loaded、444/678 overflow/遮挡与双用户截图 gate 可执行 |
| T018 | PASS | 25 Gate PG/SQLite identity/delta/privacy 合同及机器校验 PASS |
| T019 | PASS | action/frame/ack correlation safe trace 已补齐；不记录 token、cookie、正文或 VM |
| T020 | PASS | V-001 至 V-004 全绿；SharedV2Ready 成立，详见下表 |

### Formal Revisions And Hashes

| Artifact | Value |
|---|---|
| Helix V2 producer commit | `ea415a800b329947ce0757b8595df19b715f7d55` |
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

Phase 2 已完成，`SharedV2Ready` 成立；这只允许进入逐 Gate READY 工作，不等于
Native/PG/SQLite/DOM 全链路已经通过。

### SharedV2Ready Final Run

**Window**: `2026-07-23T04:58:00-07:00` 至 `2026-07-23T05:10:51-07:00`

| Validator | Command | Result | Wall Time / Artifact |
|---|---|---|---|
| V-001 | `cargo test -p helix-im --test interaction_frame_contract_test --no-default-features` | PASS，17/17 | 约 5.4s |
| V-002 | `cargo test -p helix-im` | PASS，全量 lib/integration | 约 27s |
| V-003 | `npm run message:command-api:gate && npm run message:projection-ownership && npm run message:surface-runtime:source-pin` | PASS，epoch 11，formal pin `d899a603...`，fixture 9/9 | 7.6s |
| V-004 | focused Jest + desktop E2E Jest + reducer Playwright + desktop TS + Sass | PASS，59 + 18 + 35 tests；TS clean；CSS hash 固定 | Jest 7.4s；其余见命令输出 |
| Native Rust | `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo check` | PASS，修正 safe-trace borrow 后一次完整复验 | 2m53s；既有 warnings |
| Index | `python3 scripts/verify-interaction-frame-v2-index.py` | PASS，19 Spec + 19 Plan + 19 Tasks + 25 Gate + T021-T077 | <1s |
| DB manifest | `python3 scripts/verify-interaction-frame-v2-evidence-manifest.py` | PASS，25 Gate contracts | <1s；manifest SHA-256 `18daacbf...` |

关键 artifact SHA-256：

- client `frame-evidence.ts`: `aa9765d2842ae6009ea777dd0ca9d5488c5653dbbda9a93597a10556a30ae04e`
- client `reduce-evidence.mjs`: `9a27accc9251998adaeb620197e03c52d8b04e8391c2909919968fcb4e73ba6a`
- compiled message-v4 CSS: `29b7ee8fe1066a1521eed0ed0e5b7c1dd78cc736dcce0318c6697777de07ed5f`
- evidence manifest: `18daacbf1736e01f24c4bdf5e1a5e9d13d10ba25e7b0ddbde07293e07eef60da`
- manifest verifier: `9bb871e31823f60c2df8b724fcde1690a98e8bd25a9362bceddb46d9f281221d`

`SharedV2Ready` 仅表示共享协议、工具和证据合同可进入逐 Gate 实现；尚未产生
25 Gate 的真实 PG/SQLite/双用户 DOM/截图证据，V1 删除仍被 T079-T080 硬阻塞。

## Phase 3 Focused Gate Readiness

**Captured**: `2026-07-23T05:18:00-07:00`

T021-T077 对应的生产实现与 RED/回放测试在本 Feature 创建前已经存在；本轮没有
复制状态机，而是用 V2-only writer/consumer、统一 evidence manifest 和下列 focused
回归重新收口。`READY_FOR_FINAL_E2E` 只表示可以进入同一 Native candidate，不能替代
真实 Go、PG、SQLite、FrameApplied、DOM 与截图证据。

| Dimension | Helix focused contract | client focused contract | Status |
|---|---|---|---|
| GS01 | `gs01_04_v2_sync_recovery_test` | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS02 | `gs01_04_v2_sync_recovery_test` | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS03 | `p4_sync_scheduler_test` + GS recovery | `gs01-04-sync-recovery-dom.test.ts` | READY |
| GS04 | `uc45_unknown_channel_post_test` + GS recovery | `gs01-04-sync-recovery-dom.test.ts` | READY |
| G01 | `send_test` + `f1_send_http_reconcile_test` | composer/runtime/view focused suites | READY |
| G02 | InteractionFrame codec/content contracts | typed document view test | READY |
| G04 | `g04_revoke_interaction_test` | runtime view/action contracts | READY |
| G05 | `g05_forward_interaction_test` | runtime view/forward contracts | READY |
| G06 | `g06_reaction_interaction_test` | runtime view/quick-action contracts | READY |
| G07 | `g07_urgent_interaction_test` | runtime view/urgent contracts | READY |
| G08 | `g08_schedule_create_interaction_test` | runtime view/schedule contracts | READY |
| G09 | `g09_schedule_cancel_interaction_test` | runtime view/schedule contracts | READY |
| G10 | `g10_timeline_window_interaction_test` | runtime view/anchor contracts | READY |
| G11 | `g11_locate_interaction_test` | runtime directive/view contracts | READY |
| G12 | `g12_reply_thread_interaction_test` | runtime view/thread contracts | READY |
| G13 | `g13_channel_read_interaction_test` | `g13-channel-read-interaction.test.ts` | READY |
| G14 | `g14_receipt_template_interaction_test` | `g14-receipt-template-interaction.test.ts` | READY |
| G15 | `g15_topic_interaction_test` | `g15-topic-interaction.test.ts` | READY |
| G16 | `g16_governance_interaction_test` | `g16-governance-interaction.test.ts` | READY |

### Focused Results

- Helix 单次 18-target Gate 回归：59 tests PASS，0 fail。
- client 单次 8-suite Gate/DOM 回归：首次暴露 G14 收据 Surface 未正式渲染、
  G16 close grant 未进入 menu 两个缺口；修复后 8/8 suites、38/38 tests PASS。
- G14 forbidden Frame 不再输出成员 identity，template received 与尚未接入的
  business confirmation 分开呈现。
- G16 只渲染 Helix enabled 的 close/leave grant；settings 保留现有只读 shell
  入口；disabled leave 不出现在 DOM。
- formal Helix pin 仍为 `d899a6033c90a153b5b009a62bee0e3a17dd9ad8`，
  Strict Matrix epoch 11，fixture/source-pin 9/9 一致。

**Verdict**: `19/19 READY_FOR_FINAL_E2E`。下一硬门是 T079-T080 的首次
`V2OnlyFirstRun`；在其 25/25 产品链证据全绿前，V1 deletion admission 仍为 DENY。
