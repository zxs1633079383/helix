# Tasks: InteractionFrame V2 双仓硬切

**Input**: [plan.md](plan.md)、[spec.md](spec.md)、[per-gate-spec-index.md](per-gate-spec-index.md)

**Organization**: `tasks.md` 是全局执行真源；各 Gate 的 `13-V2切换Tasks.md` 是相同 T021-T077 的就近视图。先完成共享基座，再依次迁移 Gate，最后执行首轮纯 V2、删除 V1 和删除后二次回归。

## Phase 1: Setup And Source Lock

- [X] T001 记录 Helix/cses-client/Go branch、HEAD、dirty status、四包 pin、Cargo.lock hash、Matrix hash 与 fixture hash 到 `specs/018-interaction-frame-v2-cutover/verification.md`
- [X] T002 在 `specs/018-interaction-frame-v2-cutover/discovery.md` 更新 V1 producer/consumer/normalize/raw fallback 与合法独立 `v1` inventory
- [X] T003 [P] 校验 19 个 `11/12/13` Gate 文件和 25 Gate 覆盖，在 `specs/018-interaction-frame-v2-cutover/per-gate-spec-index.md` 记录检查结果
- [X] T004 [P] 在 `specs/018-interaction-frame-v2-cutover/quickstart.md` 固定实际 focused、SCSS/CSS、Native、PG/SQLite readonly 命令和 artifact root
- [X] T005 为实现创建隔离的 Helix/cses-client worktree 并在 `specs/018-interaction-frame-v2-cutover/verification.md` 记录 branch identity；不得覆盖当前脏工作树

## Phase 2: Shared V2-only Foundation

**Checkpoint**: T006-T020 全部完成前，任何 Gate 只能写 RED test，不得标记 ready。

- [X] T006 [P] 在 `crates/helix-im/tests/interaction_frame_contract_test.rs` 增加 V1 fail-closed、schema/slot/epoch/revision/transaction/directive 的共享 RED contract
- [X] T007 [P] 在 cses-client `src/pages/message-v4/surface-runtime/__tests__/surface-contract.codec.test.ts` 增加 V2-only codec、V1/unknown schema 拒绝 RED test
- [X] T008 [P] 在 cses-client `surface-runtime/__tests__/surface-runtime.service.test.ts` 增加 epoch change/stale、整帧原子性、FrameApplied/directive 顺序 RED test
- [X] T009 [P] 在 cses-client `tests/e2e/desktop/__tests__/evidence-contract.test.ts` 增加 25-Gate profile、v1/v2/raw/ack counters 和删除前后 artifact 隔离 RED test
- [X] T010 在 `crates/helix-im/src/interaction_frame.rs`、`src/projector/mod.rs` 与 `src/surface_composer.rs` 建立唯一 V2 builder/emitter，并让生产 V1 writer fail-closed
- [X] T011 在 cses-client `surface-contract.types.ts` 与 `surface-contract.codec.ts` 将生产合同收敛为 InteractionFrameV2，移除可达 V1 union/normalize
- [X] T012 在 cses-client `surface-runtime.service.ts` 将 surfaceEpoch 纳入 slot 状态/比较，并保持 whole-frame staging/version swap
- [X] T013 在 cses-client SurfaceRuntime/Tauri bridge 与 Helix port reply 路径接通生产 `FrameApplied(transactionId, slots, result)`
- [X] T014 在 Helix fixture manifest、client fixture mirror、Strict Matrix generated contract 中将目标业务合同统一为 schemaVersion=2 和同一 source pin
- [X] T015 在 cses-client `tests/e2e/desktop/harness/uc-scenario.registry.ts` 与 `run-scenario.ts` 增加显式 25-Gate `interaction-frame-v2-cutover` profile
- [X] T016 在 cses-client `tests/e2e/desktop/harness/frame-evidence.ts`、`evidence-collector.ts` 与 reducer 增加 v1Frames/v2Frames/rawBusinessFallbacks/FrameApplied counters
- [X] T017 在 cses-client `package.json` 和 message-v4 E2E harness 增加 SCSS/CSS 编译、样式载入、444/678 overflow/遮挡 gate
- [X] T018 [P] 在 `specs/018-interaction-frame-v2-cutover/contracts/evidence-manifest-v2.md` 固定每 Gate PG/SQLite 主键、actor/observer、before/after delta 和隐私约束
- [X] T019 [P] 在 Helix/client diagnostics 增加 action/tick/effect/persist/frame/ack correlation 字段，禁止输出 token/cookie/正文/无界 VM
- [X] T020 运行 V-001 至 V-004 shared validators，并将结果、起止时间和 artifact hash 写入 `specs/018-interaction-frame-v2-cutover/verification.md`

## Phase 3: User Story 1 - 25 Gate Business Continuity (Priority: P1)

**Goal**: 每个 Gate 的 Helix writer、Angular consumer/组件和双库证据达到 `READY_FOR_FINAL_E2E`。

**Independent Test**: 每个 `13-V2切换Tasks.md` 的 RED→Helix→client/evidence 三项均 focused PASS，但不单独宣称产品完成。

### GS01

- [ ] T021 [P] [US1] 在 `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs` 与 cses-client `surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts` 增加 GS01 V2-only、PersistErr、重复/乱序和 FrameApplied RED test
- [ ] T022 [US1] 在 `crates/helix-im/src/sync/{session,increment,flow}.rs` 与 `src/surface_composer.rs` 实现 GS01 matching PersistOk 后 dialog/unread V2 snapshot
- [ ] T023 [US1] 在 cses-client `surface-runtime/` 和 Native frame evidence 登记 GS01 V2 apply/ack/DOM、PG 与 SQLite delta

### GS02

- [ ] T024 [P] [US1] 在 Helix GS recovery test 和 cses-client 恢复 DOM test 增加 full/mixed/no_change/满批/重复/PersistErr RED test
- [ ] T025 [US1] 在 `crates/helix-im/src/sync/{explicit,flow}.rs` 与 `src/channel/gate/{flow,commit}.rs` 实现连续 batch、message+cursor atomic persist 和 V2 compose
- [ ] T026 [US1] 在 Native evidence collector 登记 GS02 fromSeq/nextSeq、PG event、SQLite cursor/message、FrameApplied 与 DOM

### GS03

- [ ] T027 [P] [US1] 在 `src/send/pong_compensate_tests.rs`、`tests/p4_sync_scheduler_test.rs` 和 client sync tests 增加普通 pong/重复 gap/并发/no_change/PersistErr RED test
- [ ] T028 [US1] 在 `src/send/pong_compensate.rs` 与 `src/sync/scheduler.rs` 实现 only-gap 去重、有界队列/重试和 post-persist V2 compose
- [ ] T029 [US1] 在 Native diagnostics/evidence collector 登记 gap decision、queue、PG head、SQLite cursor、FrameApplied 和非 gap 零请求

### GS04

- [ ] T030 [P] [US1] 在 `tests/uc45_unknown_channel_post_test.rs`、GS recovery test 和 client DOM test 增加 unknown/forbidden/PersistErr/pre-commit no-send RED test
- [ ] T031 [US1] 在 `src/channel/gate/flow.rs`、`src/sync/flow.rs`、`src/surface_composer.rs` 实现 hydrate write-set 和 summary/composer/timeline V2 bundle
- [ ] T032 [US1] 在 client facade/components/evidence collector 验证 GS04 PG/SQLite identity、binding 权限、FrameApplied 和三 Surface DOM

### G01-G02

- [ ] T033 [P] [US1] 在 `tests/send_test.rs`、`f1_send_http_reconcile_test.rs` 与 client composer/runtime tests 增加 G01 binding/HTTP2xx/duplicate/late/PersistErr RED test
- [ ] T034 [US1] 在 `src/module/action.rs`、`src/send/{flow,pending,reconcile}.rs`、`src/surface_composer.rs` 实现 G01 temporary→canonical V2-only flow
- [ ] T035 [US1] 在 client composer/projection/runtime/evidence collector 验证 G01 clear-after-apply、双用户 DOM 和双库 identity
- [ ] T036 [P] [US1] 在 Helix contract/send tests 与 client content/codec tests 增加 G02 DOCUMENT typed/unknown/missing/failure RED test
- [ ] T037 [US1] 在 `src/interaction_frame.rs` 和 send content normalize 实现 G02 typed document VM 并复用 G01 settle
- [ ] T038 [US1] 在 client contract/document component/evidence collector 删除 raw props business branch 并登记 G02 双库/双用户证据

### G04-G09

- [ ] T039 [P] [US1] 在 Helix/client G04 tests 增加 online/offline/权限/重复/PersistErr RED test
- [ ] T040 [US1] 在 Helix revoke normalize/persist/composer 实现 G04/G04a 共同撤回 V2 事实
- [ ] T041 [US1] 在 client action/message/thread/evidence 验证 G04 旧 binding 失效和双库/双用户撤回态
- [ ] T042 [P] [US1] 在 Helix/client G05 tests 增加多目标/部分失败/重复/late RED test
- [ ] T043 [US1] 在 `src/forward.rs`、render-ready forward 和 composer 实现 G05 target-scoped V2 终局
- [ ] T044 [US1] 在 client forward overlay/runtime/evidence 验证 G05 逐目标双库/observer terminal
- [ ] T045 [P] [US1] 在 Helix/client G06 tests 增加 reaction 并发/重复/切换/拒绝/PersistErr RED test
- [ ] T046 [US1] 在 Helix quick-reply normalize、timeline/thread projector/composer 实现 G06 权威 V2 聚合
- [ ] T047 [US1] 在 client quick-actions/action-menu/evidence 验证 G06 双库聚合与双用户状态
- [ ] T048 [P] [US1] 在 Helix/client G07 tests 增加 urgent/confirm/非目标/重复/PersistErr RED test
- [ ] T049 [US1] 在 Helix urgent/receipt projector/composer 实现 G07 分离 binding 和 viewer V2 Frame
- [ ] T050 [US1] 在 client urgent/receipt/evidence 验证 G07 PG/SQLite receipt 与双用户按钮/人数
- [ ] T051 [P] [US1] 在 Helix/client G08 tests 增加 owner/observer/过去时间/重复/PersistErr RED test
- [ ] T052 [US1] 在 Helix schedule create/hydrate/storage/composer 实现 G08 owner-scoped V2 Frame
- [ ] T053 [US1] 在 client schedule/evidence 验证 G08 双库、owner banner 和 observer no-leak
- [ ] T054 [P] [US1] 在 Helix/client G09 tests 增加跨 actor/重复/失败/PersistErr RED test
- [ ] T055 [US1] 在 Helix schedule cancel/storage/composer 实现 G09 owner terminal V2 Frame
- [ ] T056 [US1] 在 client schedule/evidence 验证 G09 双库 terminal、owner DOM 和重复零 mutation

### G10-G16

- [ ] T057 [P] [US1] 在 Helix/client G10 tests 增加 complete/incomplete/end/并发/stale/PersistErr RED test
- [ ] T058 [US1] 在 Helix render-ready/timeline/composer 实现 G10 bounded local-first/backfill V2 window
- [ ] T059 [US1] 在 client timeline/anchor/evidence 验证 G10 零出网、双库 window、DOM 去重和 anchor
- [ ] T060 [P] [US1] 在 Helix/client G11 tests 增加 local/remote/not-found/stale/PersistErr RED test
- [ ] T061 [US1] 在 Helix locate/timeline/composer 实现 G11 context Frame 和 post-commit reveal
- [ ] T062 [US1] 在 client runtime/anchor/evidence 验证 G11 target、FrameApplied 顺序和 DOM focus
- [ ] T063 [P] [US1] 在 Helix/client G12 tests 增加分页/缺 root/循环/重复/PersistErr RED test
- [ ] T064 [US1] 在 Helix replies/thread/timeline/composer 实现 G12 三锚 graph V2 compose
- [ ] T065 [US1] 在 client reply/thread/evidence 验证 G12 双库三锚、共享 identity 和 DOM
- [ ] T066 [P] [US1] 在 Helix/client G13 tests 增加单调/重复/节流/viewer/PersistErr RED test
- [ ] T067 [US1] 在 Helix read/projectors/composer 实现 G13 boundary PersistOk 后三 Surface V2 update
- [ ] T068 [US1] 在 client read/dialog/evidence 验证 G13 双库 boundary、双用户未读和零乐观清零
- [ ] T069 [P] [US1] 在 Helix/client G14 tests 增加重复/跨 post/viewer/语义混写/PersistErr RED test
- [ ] T070 [US1] 在 Helix receipt/template projectors/composer 实现 G14/G14a 分离 V2 状态
- [ ] T071 [US1] 在 client receipt/template/evidence 验证 G14 双库、author/reader DOM 和术语
- [ ] T072 [P] [US1] 在 Helix/client G15 tests 增加 topic/channel/重复/无效成员/PersistErr RED test
- [ ] T073 [US1] 在 Helix channel-create/topic/reply projectors/composer 实现 G15/G15a entity bundle V2 compose
- [ ] T074 [US1] 在 client create-topic/chat/evidence 验证 G15 双库 identity、membership 和首批 Surface
- [ ] T075 [P] [US1] 在 Helix/client G16 tests 增加 close/leave/remove/settings/conflict/PersistErr RED test
- [ ] T076 [US1] 在 Helix channel admin/member/settings projectors/composer 实现 G16 viewer-specific terminal V2 Frame
- [ ] T077 [US1] 在 client governance/runtime/evidence 验证 G16 双库 lifecycle 与 actor/target/observer DOM

## Phase 4: User Story 2 - V2 Is The Only Maintained Protocol (Priority: P1)

**Goal**: 先证明运行时纯 V2，再物理删除 V1 并保持回归。

- [ ] T078 [US2] 将 T021-T077 的 focused evidence 汇总到 `specs/018-interaction-frame-v2-cutover/verification.md`，要求 19/19 READY 且 pin/hash 一致
- [ ] T079 [US2] 在 cses-client 构建单一 `V2OnlyFirstRun` Native candidate，并通过 25-Gate profile 连续执行所有 Gate
- [ ] T080 [US2] 运行 collect/generate-matrix/reduce，要求 25/25、fail=0、skip=0、v1Frames=0、rawBusinessFallbacks=0、FrameApplied 完整
- [ ] T081 [US2] 根据 `discovery.md` 和首轮 evidence 审批 V1 deletion inventory，区分 InteractionFrame/RenderNode V1 与合法独立版本
- [ ] T082 [US2] 在 Helix `interaction_frame.rs`、projector/emitter、fixtures/tests 中物理删除已批准 V1 writer/contract
- [ ] T083 [US2] 在 cses-client `surface-contract.*`、legacy render/projection、fixtures/tests 中物理删除已批准 V1 reader/normalize/fallback
- [ ] T084 [US2] 更新 Strict Matrix、fixture manifests、source pin 与 static scans，证明删除后可达 V1 producer/consumer 为 0

## Phase 5: User Story 3 - First-Hop Evidence Diagnosis (Priority: P2)

**Goal**: 任一失败都能定位 action/authority/persist/frame/apply/DOM 的第一处偏离。

- [ ] T085 [P] [US3] 在 `tests/e2e/desktop/harness/frame-evidence.ts` 固定 action/tick/effect/persist/frame/ack correlation schema
- [ ] T086 [P] [US3] 在 `scripts/message-real-chain/collect-evidence.mjs` 验证每 Gate required facets、run window 和 artifact hash
- [ ] T087 [P] [US3] 在 evidence contract tests 注入 V1、raw fallback、PersistErr、epoch/revision gap、duplicate transaction 并证明 fail-closed
- [ ] T088 [US3] 为 25 Gate 在 evidence manifest/collector 登记 PG/SQLite 表、主键、actor/observer 和 before/after delta 查询
- [ ] T089 [US3] 将 SCSS/CSS、444/678 bounding-box/overflow 和双用户截图接入每 Gate visual facet
- [ ] T090 [US3] 在 reducer 输出每 Gate 第一失败 hop、FAIL/BLOCKED 原因和 command start/end/wall-time/retry/wait
- [ ] T091 [US3] 审查首轮 V2 evidence 是否能从每个 Gate 输入追到 FrameApplied/DOM；缺任一面不得进入 deletion

## Phase 6: Post-deletion Regression And Closure

- [ ] T092 创建新的 `PostDeletionRun` candidateId/runId/evidenceEpoch/artifact root，禁止读取首轮 artifact
- [ ] T093 [P] 运行 Helix contract/full tests、client codec/runtime/component/fixture parity 和 SCSS/CSS gate
- [ ] T094 在新 Native candidate 连续重跑 25 Gate，并重新采集双用户、Go、PG、SQLite、Frame、ack、DOM、截图
- [ ] T095 运行删除后二次 reducer，要求 25/25、fail=0、skip=0、v1/raw=0 且 source/hash 全一致
- [ ] T096 将命令、起止时间、耗时、artifact hash、删除清单和最终 verdict 写入 `verification.md` 并运行 after_implement R4

## Dependencies And Execution Order

```text
T001-T005
  -> T006-T020
  -> T021-T032
  -> T033-T038
  -> T039-T056
  -> T057-T077
  -> T078-T080
  -> T081-T084
  -> T085-T091 (工具实现可在 Gate 纵切期间并行，T091 依赖 T080)
  -> T092-T096
```

- 共享基座完成后，不同 Gate 的 RED tests 可并行；共享 `surface_composer.rs`、codec/runtime 和 evidence collector 的实现必须串行合并。
- 任一 Gate focused 失败不阻塞其他独立 RED test，但阻塞 T078。
- 首轮 V2-only E2E 是 deletion 的硬前置；删除后必须新 candidate 二次回归。

## Implementation Strategy

1. MVP：T001-T032，先证明 shared V2-only + GS01-GS04 recovery。
2. 增量：按 G01-G02、G04-G09、G10-G16 批次推进，每批完成 focused readiness。
3. 集成：一次 Native build 连续跑 25 Gate。
4. 清退：首轮纯 V2 全绿后才删除 V1。
5. 关闭：删除后新 evidence epoch 全绿且 R4 PASS。

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-015 | I-001 | `verification.md` | R1/R2 PASS | pin/hash inspection | source lock recorded |
| T002 | FR-020 | I-004 | `discovery.md` | T001 | inventory scan | V1/legal-v1 classified |
| T003 | FR-001 | I-003 | per-gate index | T001 | 19/25 count check | coverage 100% |
| T004 | FR-018 | V-001..V-008 | `quickstart.md` | T001 | command existence check | runnable commands fixed |
| T005 | FR-026 | I-001 | worktree records | T001 | branch identity script | isolated worktrees ready |
| T006 | FR-004 | A-001 | Helix contract test | T001-T005 | V-001 RED | V1 writer test fails |
| T007 | FR-003 | A-001 | client codec test | T001-T005 | focused Jest RED | V1 reader test fails |
| T008 | FR-002 | S-001 | runtime test | T001-T005 | focused Jest RED | epoch/ack tests fail |
| T009 | FR-017 | V-006 | evidence test | T001-T005 | node test RED | counters/profile tests fail |
| T010 | FR-004 | A-001 | Helix frame builder | T006 | V-001 | V2-only writer PASS |
| T011 | FR-003 | A-001 | client codec/types | T007 | V-004 | V2-only reader PASS |
| T012 | FR-010 | S-001 | client slot state | T008 | V-004 | epoch/revision PASS |
| T013 | FR-012 | A-001 | ack bridge | T008,T012 | V-004 | production ack PASS |
| T014 | FR-015 | A-001 | matrix/fixtures | T010-T013 | parity tests | pins/contracts equal |
| T015 | FR-016 | A-003 | Native profile | T009 | profile test | 25 Gate selected |
| T016 | FR-017 | V-006 | evidence/reducer | T009 | evidence tests | counters enforced |
| T017 | FR-018 | V-004 | CSS/visual gate | T004 | style/e2e tests | layout gate executable |
| T018 | FR-018 | V-007 | evidence contract | T003 | manifest lint | 25 DB contracts present |
| T019 | FR-024 | A-005 | diagnostics | T010-T013 | correlation test | hop ids complete |
| T020 | FR-002 | S-001 | shared verification | T010-T019 | V-001..V-004 | SharedV2Ready |
| T021 | FR-018 | A-002 | GS01 tests | T020 | focused RED | negative cases fail |
| T022 | FR-008 | A-002 | GS01 Helix | T021 | GS replay | post-persist V2 PASS |
| T023 | FR-013 | A-002 | GS01 client/evidence | T022 | DOM/evidence tests | GS01 READY |
| T024 | FR-018 | A-002 | GS02 tests | T020 | focused RED | batch cases fail |
| T025 | FR-008 | A-002 | GS02 Helix | T024 | GS replay | atomic cursor PASS |
| T026 | FR-024 | A-002 | GS02 evidence | T025 | facet check | GS02 READY |
| T027 | FR-018 | A-002 | GS03 tests | T020 | focused RED | gap cases fail |
| T028 | FR-007 | A-002 | GS03 Helix | T027 | scheduler tests | bounded gap PASS |
| T029 | FR-024 | A-002 | GS03 evidence | T028 | diagnostics check | GS03 READY |
| T030 | FR-018 | A-002 | GS04 tests | T020 | focused RED | hydrate cases fail |
| T031 | FR-008 | A-002 | GS04 Helix | T030 | recovery replay | bundle PASS |
| T032 | FR-013 | A-002 | GS04 client/evidence | T031 | DOM/facet check | GS04 READY |
| T033 | FR-018 | A-002 | G01 tests | T032 | focused RED | send cases fail |
| T034 | FR-005 | A-002 | G01 Helix | T033 | send tests | canonical settle PASS |
| T035 | FR-013 | A-002 | G01 client/evidence | T034 | DOM/DB facets | G01 READY |
| T036 | FR-018 | A-002 | G02 tests | T034 | focused RED | document cases fail |
| T037 | FR-009 | A-002 | G02 Helix | T036 | contract/send tests | typed VM PASS |
| T038 | FR-011 | A-002 | G02 client/evidence | T037 | component/facet tests | G02 READY |
| T039 | FR-018 | A-002 | G04 tests | T020 | focused RED | revoke cases fail |
| T040 | FR-008 | A-002 | G04 Helix | T039 | G04 replay | common fact PASS |
| T041 | FR-013 | A-002 | G04 client/evidence | T040 | G04 DOM/facets | G04 READY |
| T042 | FR-018 | A-002 | G05 tests | T020 | focused RED | forward cases fail |
| T043 | FR-008 | A-002 | G05 Helix | T042 | G05 replay | per-target PASS |
| T044 | FR-013 | A-002 | G05 client/evidence | T043 | G05 DOM/facets | G05 READY |
| T045 | FR-018 | A-002 | G06 tests | T020 | focused RED | reaction cases fail |
| T046 | FR-008 | A-002 | G06 Helix | T045 | G06 replay | aggregate PASS |
| T047 | FR-013 | A-002 | G06 client/evidence | T046 | G06 DOM/facets | G06 READY |
| T048 | FR-018 | A-002 | G07 tests | T020 | focused RED | urgent cases fail |
| T049 | FR-005 | A-002 | G07 Helix | T048 | G07 replay | viewer frame PASS |
| T050 | FR-013 | A-002 | G07 client/evidence | T049 | G07 DOM/facets | G07 READY |
| T051 | FR-018 | A-002 | G08 tests | T020 | focused RED | schedule cases fail |
| T052 | FR-008 | A-002 | G08 Helix | T051 | G08 replay | owner frame PASS |
| T053 | FR-013 | A-002 | G08 client/evidence | T052 | G08 DOM/facets | G08 READY |
| T054 | FR-018 | A-002 | G09 tests | T020 | focused RED | cancel cases fail |
| T055 | FR-008 | A-002 | G09 Helix | T054 | G09 replay | terminal PASS |
| T056 | FR-013 | A-002 | G09 client/evidence | T055 | G09 DOM/facets | G09 READY |
| T057 | FR-018 | A-002 | G10 tests | T020 | focused RED | window cases fail |
| T058 | FR-010 | A-002 | G10 Helix | T057 | G10 replay | bounded window PASS |
| T059 | FR-013 | A-002 | G10 client/evidence | T058 | anchor/facets | G10 READY |
| T060 | FR-018 | A-002 | G11 tests | T020 | focused RED | locate cases fail |
| T061 | FR-009 | A-002 | G11 Helix | T060 | G11 replay | commit/reveal PASS |
| T062 | FR-012 | A-002 | G11 client/evidence | T061 | directive/facets | G11 READY |
| T063 | FR-018 | A-002 | G12 tests | T020 | focused RED | graph cases fail |
| T064 | FR-010 | A-002 | G12 Helix | T063 | G12 replay | graph PASS |
| T065 | FR-013 | A-002 | G12 client/evidence | T064 | drawer/facets | G12 READY |
| T066 | FR-018 | A-002 | G13 tests | T020 | focused RED | read cases fail |
| T067 | FR-008 | A-002 | G13 Helix | T066 | G13 replay | boundary PASS |
| T068 | FR-013 | A-002 | G13 client/evidence | T067 | read/facets | G13 READY |
| T069 | FR-018 | A-002 | G14 tests | T020 | focused RED | receipt cases fail |
| T070 | FR-009 | A-002 | G14 Helix | T069 | G14 replay | split semantics PASS |
| T071 | FR-013 | A-002 | G14 client/evidence | T070 | receipt/facets | G14 READY |
| T072 | FR-018 | A-002 | G15 tests | T020 | focused RED | creation cases fail |
| T073 | FR-008 | A-002 | G15 Helix | T072 | G15 replay | entity bundle PASS |
| T074 | FR-013 | A-002 | G15 client/evidence | T073 | creation/facets | G15 READY |
| T075 | FR-018 | A-002 | G16 tests | T020 | focused RED | governance cases fail |
| T076 | FR-009 | A-002 | G16 Helix | T075 | G16 replay | terminal frames PASS |
| T077 | FR-013 | A-002 | G16 client/evidence | T076 | governance/facets | G16 READY |
| T078 | FR-026 | S-003 | readiness summary | T021-T077 | 19/19 check | all GateReady |
| T079 | FR-016 | A-003 | first candidate | T078 | build/profile | V2 candidate running |
| T080 | FR-017 | S-004 | first evidence | T079 | V-005,V-006,V-007 | FirstE2EPass |
| T081 | FR-020 | A-004 | deletion inventory | T080 | inventory review | deletion admitted |
| T082 | FR-020 | S-005 | Helix V1 delete | T081 | Rust/static tests | Helix V1 absent |
| T083 | FR-020 | S-005 | client V1 delete | T081 | Jest/static tests | client V1 absent |
| T084 | FR-021 | S-005 | contracts/pins | T082,T083 | parity/static scan | legal versions preserved |
| T085 | FR-024 | V-006 | frame evidence | T016 | schema tests | hop schema complete |
| T086 | FR-018 | V-006 | collector | T018 | collector tests | facets enforced |
| T087 | FR-002 | A-005 | negative evidence | T016 | evidence tests | failures fail-closed |
| T088 | FR-018 | V-007 | DB facets | T018 | readonly dry-run | 25 DB queries valid |
| T089 | FR-018 | V-004 | visual facets | T017 | CSS/visual tests | 25 visual facets valid |
| T090 | FR-024 | A-005 | reducer report | T085-T089 | reducer tests | first hop reported |
| T091 | FR-023 | S-004 | first-run review | T080,T090 | evidence audit | no inherited Green |
| T092 | FR-022 | S-006 | new evidence epoch | T084,T091 | isolation test | fresh candidate |
| T093 | FR-022 | V-008 | focused regression | T092 | Rust/Jest/CSS | focused PASS |
| T094 | FR-022 | A-004 | second Native run | T093 | V-005,V-007 | 25 Gate artifacts |
| T095 | FR-017 | S-006 | second reducer | T094 | V-006,V-008 | 25/25 V2-only PASS |
| T096 | FR-025 | S-006 | closure evidence | T095 | loop R4 | feature CLOSED |
