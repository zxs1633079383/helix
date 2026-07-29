# Tasks: SPEC-IFV2-G01-在线发送

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md), [data-model.md](./data-model.md), [G01 V2 contract](./contracts/interaction-frame-v2-g01.md), [quickstart.md](./quickstart.md)

**Tests**: Required by FR-011 and SC-004. Tests are written first for every user story and must be observed failing against the legacy behaviour before the corresponding implementation changes.

**Organization**: 001 GS01-GS04 is the shared V2 owner. Phase 0 is a hard admission gate: before T001 passes, every source/fixture/PC task below is blocked, no implementation worktree may be created or used, and no Cargo/Jest/real-chain command may run. After admission, Phase 3-5 are independently testable increments for spec User Stories P1-P3. G01 source and V4 consumer form one hard-cut increment: never introduce a raw emit dual-write or a V4 raw fallback; do not re-implement 001's nine-Surface grammar, Frame Cache, generic fixture manifest, or hydration/composer baseline.

## Phase 0: GS01-GS04 Baseline Admission

**Purpose**: Consume, rather than recreate, the accepted V2 shared baseline. This phase is documentation/read-only only.

- [ ] T001 Read `specs/001-gs-sync-recovery/contracts/sync-recovery-evidence.md` and verify that 001 T050 records the **current source pin** with Rust, PC DOM, mobile fixture, and SQLite/PG readonly evidence all explicitly `PASS`; record the pin, evidence locations, G01 `corrKey` packet layout, and 30-minute progress-report template in `specs/002-ifv2-g01-online-send/quickstart.md`. Any missing/old-pin/non-PASS verdict leaves T002-T041 blocked and forbids creating/using either implementation worktree or running implementation commands.

**Checkpoint**: Only a current-pin, four-plane 001 T050 `PASS` unlocks G01 implementation. “001 is running”, a design document, a historical pin, or one green fixture is not admission.

---

## Phase 1: G01 Extension Inputs

**Purpose**: Add only G01 cases and reproducible evidence metadata to the finalized 001 artifacts; do not change common V2 grammar.

- [ ] T002 [P] Add only G01 Sending/Sent/Failed, HTTP-2xx, duplicate/late-echo, and corrupt-frame cases to the **finalized 001 fixture manifest grammar** in `crates/helix-im/tests/fixtures/uc01-16-surface-runtime/manifest.json`; do not redefine its nine-Surface schema or generic manifest fields.
- [ ] T003 [P] Add matching G01 generated-wire parity cases to the **finalized 001 PC fixture grammar** in `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/__fixtures__/uc01-16-surface-runtime/fixture-manifest.json`; do not fork a second registry.
- [ ] T004 Verify exact focused Jest/desktop commands and evidence artifact directories against `CSES_CLIENT_WORKTREE/package.json`, then update the executable command section in `specs/002-ifv2-g01-online-send/quickstart.md` with the admitted 001 source pin.

---

## Phase 2: G01 Contract Extension On Accepted V2 Baseline

**Purpose**: Extend accepted V2 only with G01 text content, delivery and action semantics. Do not replace shared typed V2, the Frame Cache algorithm, nine-Surface registry or hydration/composer availability owned by 001.

- [ ] T005 Add contract-first failing **G01-specific** cases against the accepted 001 V2 contract: text input/content, `MessageRowVmV2.delivery`, causation/directive fields, and unknown G01 content rejection in `crates/helix-im/tests/interaction_frame_contract_test.rs`; do not redefine composer, runtime/slot or common frame fields.
- [ ] T006 [P] Add equivalent PC G01 content/delivery codec tests, including unknown G01 discriminator rejection and invalid-frame cache unchanged, in `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/__tests__/surface-contract.codec.test.ts`; reuse 001's decoder/cache contract.
- [ ] T007 [P] Add G01 integration tests proving the existing 001 cache emits `FrameApplied` before `ClearComposerDraft`, and directive failure does not roll back a committed frame, in `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts`; do not alter whole-frame commit ownership.
- [ ] T008 Extend the accepted V2 message content/message-row model only with typed G01 text content and `DeliveryVm`, retaining 001's shared Surface/Frame Cache baseline and explicit V1 compatibility isolation in `crates/helix-im/src/interaction_frame.rs`.
- [ ] T009 Extend the existing PC V2 codec only with G01 typed content/delivery and camelCase public fields in `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/surface-contract.types.ts` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/surface-contract.codec.ts`; do not redefine scopes, slots, schema or caching.
- [ ] T010 Wire G01 directive handling through the existing 001 `FrameApplied` API in `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/surface-runtime.service.ts`; no generic whole-frame algorithm change.
- [ ] T011 Append G01 cases to the accepted Helix-to-PC fixture parity assertion in `crates/helix-im/tests/interaction_frame_fixture_manifest_test.rs` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.fixture-parity.test.ts`; do not reassert/redefine the nine-Surface contract.

**Checkpoint**: A malformed G01 V2 frame changes no PC cache slot; valid G01 cases decode on both sides using the accepted 001 V2 baseline. `message.composer` remains a distinct channel-scoped Surface, not a timeline field.

---

## Phase 3: User Story 1 - 发送者看到单一且可信的消息状态 (Priority: P1) MVP

**Goal**: A valid plain-text action produces a persisted `Sending` row, stays Sending on HTTP 2xx, then converges one logical row to `Sent` only after authoritative canonical persistence.

**Independent Test**: In an isolated channel, one valid action yields `accepted -> PersistAtomic -> PersistOk -> Sending frame -> HTTP 2xx (still Sending) -> canonical PersistOk -> Sent frame`; sender DOM has exactly one logical row and no raw projection event was consumed.

### Tests for User Story 1

- [ ] T012 [P] [US1] Add failing Tick/Effect ordering tests for valid composer binding, temporary atomic persist, no pre-persist Sending Frame, and HTTP 2xx remaining Sending in `crates/helix-im/tests/send_test.rs`.
- [ ] T013 [P] [US1] Add failing canonical settle/duplicate/late-echo golden cases proving one temporary-to-server row transition in `crates/helix-im/tests/f1_send_http_reconcile_test.rs` and `crates/helix-im/tests/f5_golden_replay.rs`.
- [ ] T014 [P] [US1] Add PC ingress/cache tests that accept only `im:interaction:frame` V2 for G01 and reject any raw send projection source in `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/projection-bus.service.test.ts` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/surface-runtime-hard-cut.test.ts`.
- [ ] T015 [P] [US1] Add composer/DOM tests for Return-submit recovery draft, `Sending`, unchanged-on-HTTP-2xx, `Sent`, one row, and post-apply `ClearComposerDraft` in `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/message-ui-composer.intent.test.ts` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts`.

### Implementation for User Story 1

- [ ] T016 [US1] Consume the 001-issued `message.composer@Channel` `message.create` capability and validate its opaque binding with fixed action kind `im.send-message` plus actor/scope/epoch/revision/text/reply; do not issue a binding from channel hydration/availability or accept client-chosen action kind/channel/user/temporary IDs in `crates/helix-im/src/module/action.rs`.
- [ ] T017 [US1] Extend the pending-send aggregate with request-scoped correlation, temporary-to-canonical identity settlement, delivery state, and idempotent duplicate/late canonical guards in `crates/helix-im/src/send/pending.rs` and `crates/helix-im/src/send/pending/persistence.rs`.
- [ ] T018 [US1] Replace the G01 command flow with valid binding -> Tick -> `PersistAtomic(temporary message + PendingSend)` and ensure no Sending mutation is emitted before `PersistOk` in `crates/helix-im/src/send/flow.rs`.
- [ ] T019 [US1] Implement the `PersistOk` fact-to-surface composition for `timeline@Channel`, changed `left-dialog-list@Viewer`, conditional attached reply thread, and `ClearComposerDraft(requestId)` in `crates/helix-im/src/projector/action.rs` and `crates/helix-im/src/surface_composer.rs`, reusing 001's summary/composer/runtime bundle rather than rebuilding it.
- [ ] T020 [US1] Replace `posts/create` HTTP-success-to-Sent reconciliation with transport acceptance only; settle to canonical `Sent` only from matching WS/sync fact plus canonical `PersistOk` in `crates/helix-im/src/send/reconcile.rs` and `crates/helix-im/src/send/pending.rs`.
- [ ] T021 [US1] Establish G01 hard-cut readiness: make the new V2 flow emit only InteractionFrame V2 with no producer dual-write or PC raw fallback; record the exact legacy raw producer/fixture/subscription deletion inventory, but do not physically delete it before T039 approval, in `crates/helix-im/src/send/flow.rs`, `crates/helix-im/tests/send_test.rs`, and `crates/helix-im/tests/f5_golden_replay.rs`.
- [ ] T022 [US1] Route `im:interaction:frame` G01 bytes through the existing 001 typed V2 decode/cache apply and FrameApplied/directive acknowledgements without raw reducer fallback in `CSES_CLIENT_WORKTREE/src/pages/message-v4/projection-bus.service.ts` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/projection-store.service.ts`; do not take ownership of the generic cache service.
- [ ] T023 [US1] Implement `message.composer` local overlay/recovery draft handling and send intent construction from the binding, then render typed `DeliveryVm` in `CSES_CLIENT_WORKTREE/src/pages/message-v4/composer-store.service.ts`, `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/message-ui-composer.component.ts`, and `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/message-surface-runtime-view.component.ts`.
- [ ] T024 [US1] Run and record the focused Rust/PC suites plus `npm run test:message-v4:gate` from `CSES_CLIENT_WORKTREE/package.json` in `specs/002-ifv2-g01-online-send/quickstart.md` with start/end/duration and pass/fail evidence.

**Checkpoint**: US1 is independently complete when one sender sees exactly one `Sending -> Sent` row; HTTP success alone cannot create Sent; the V2 path has no raw emit producer/consumer route, and any legacy raw deletion waits for T039 approval.

---

## Phase 4: User Story 2 - 失败不会丢失输入或误导用户 (Priority: P2)

**Goal**: Invalid admission/persist failure restores only its draft; post-Sending remote failure persists and renders the same logical row as Failed with a correctly scoped retry action.

**Independent Test**: Force stale binding, invalid content, temporary PersistErr, timeout, retryable/nonretryable remote error, duplicate/late confirmation, and bad frame; no case claims Sent without authority or changes unrelated rows/slots.

### Tests for User Story 2

- [ ] T025 [P] [US2] Add failing stale binding, invalid text/reply, PersistErr recovery-draft, timeout, remote failure, retryability, duplicate/late confirmation, and retry idempotency tests in `crates/helix-im/tests/send_test.rs` and `crates/helix-im/tests/f1_send_http_reconcile_test.rs`.
- [ ] T026 [P] [US2] Add failing failure-frame golden fixtures proving same-row `Failed`, no pre-persist row, no fake Sent, and no stale retry binding in `crates/helix-im/tests/f5_golden_replay.rs` and `crates/helix-im/tests/fixtures/uc01-16-surface-runtime/manifest.json`.
- [ ] T027 [P] [US2] Add PC DOM/cache tests for recovery draft, `Failed` display, retry availability, corrupt frame atomic rejection, and resync request in `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/message-ui-composer.intent.test.ts`, `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/__tests__/message-surface-runtime-view.component.test.ts`, and `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts`.

### Implementation for User Story 2

- [ ] T028 [US2] Implement rejected-action and pre-Sending PersistErr paths that preserve/restore only the request-scoped recovery draft and create no message mutation in `crates/helix-im/src/module/action.rs` and `crates/helix-im/src/projector/action.rs`.
- [ ] T029 [US2] Implement durable timeout/remote-error delivery transitions, same-row `Failed { reason, retryable }`, and row-local retry binding renewal in `crates/helix-im/src/send/pending.rs`, `crates/helix-im/src/send/reconcile.rs`, and `crates/helix-im/src/projector/action.rs`.
- [ ] T030 [US2] Render rejected/recovery and typed failed/retry overlay states without writing business VM/cache from the component in `CSES_CLIENT_WORKTREE/src/pages/message-v4/composer-store.service.ts`, `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/message-ui-composer.component.ts`, and `CSES_CLIENT_WORKTREE/src/pages/message-v4/components/message-surface-action-bindings.component.ts`.
- [ ] T031 [US2] Run the focused Rust and PC failure suites and record one deliberately corrupt-frame counterexample in `specs/002-ifv2-g01-online-send/quickstart.md`.

**Checkpoint**: US2 is independently complete when rejection/persist errors lose no input, remote failures retain one failed row, and bad frames leave all cache slots unchanged.

---

## Phase 5: User Story 3 - 接收者和验证者独立确认结果 (Priority: P3)

**Goal**: Sender temporary state is private; receiver receives only its own persisted canonical update; a bounded evidence run joins action, storage, remote facts and both DOMs by one correlation.

**Independent Test**: Two accounts share an isolated channel: B cannot observe A’s Sending row; after B’s own canonical inbound/sync B sees canonical message/preview/unread where applicable; all required evidence planes agree.

### Tests for User Story 3

- [ ] T032 [P] [US3] Add Helix replay/fixture tests proving sender-only temporary rows, receiver canonical-only frames, conditional unread mutation, and `corrKey` propagation through action/effect/persist/frame in `crates/helix-im/tests/replay.rs` and `crates/helix-im/tests/real_go_frames_test.rs`.
- [ ] T033 [P] [US3] Add dual-profile desktop DOM assertions that B cannot see A’s Sending row and later sees the canonical row/preview in `CSES_CLIENT_WORKTREE/tests/e2e/desktop/run-g01-gs2.ts` and `CSES_CLIENT_WORKTREE/tests/e2e/desktop/jest.config.js`.

### Implementation and verification for User Story 3

- [ ] T034 [US3] Keep sender/receiver viewer scope separate in canonical inbound/sync composition and emit receiver timeline/dialog/thread/unread mutations only from receiver-persisted facts in `crates/helix-im/src/ws/frame.rs`, `crates/helix-im/src/projector/action.rs`, and `crates/helix-im/src/surface_composer.rs`.
- [ ] T035 [US3] Carry `corrKey`/`requestId` through PC ingress, cache diagnostics, FrameApplied/DirectiveApplied, and DOM evidence hooks in `CSES_CLIENT_WORKTREE/src/pages/message-v4/projection-store.service.ts`, `CSES_CLIENT_WORKTREE/src/pages/message-v4/surface-runtime/surface-runtime.service.ts`, and `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/message-runtime-diagnostics.service.test.ts`.
- [ ] T036 [US3] Execute the normal two-user G01 desktop scenario using `CSES_CLIENT_WORKTREE/package.json` `message:desktop:run:g01-gs2`, retain sender/receiver DOM and three-hop logs, and index the artifacts in `specs/002-ifv2-g01-online-send/quickstart.md`.
- [ ] T037 [US3] Query only bounded local SQLite and remote PG facts with `mcp__helix_local_sqlite_ro` and `mcp__cses_pre_postgres_ro`, then record correlation-aligned results or an explicit blocked/inconclusive verdict in `specs/002-ifv2-g01-online-send/quickstart.md`.

**Checkpoint**: US3 is independently complete when no sender temporary state leaks, all available evidence planes agree by correlation, and unavailable/not-queryable data blocks rather than fabricates pass.

---

## Phase 6: Polish And Cross-Cutting Acceptance

**Purpose**: Close regression, observability, documentation and physical-deletion evidence without widening G01 into G02-G16c.

- [ ] T038 [P] Add structured `corrKey` hop assertions for action ingress, command Tick, Persist effect/reply, HTTP/WS inbound, SurfaceComposer, frame output, cache apply, FrameApplied and directive outcome in `crates/helix-im/tests/interaction_frame_contract_test.rs` and `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/projection-store.service.test.ts`.
- [ ] T039 Re-run the V2 documentation/27-Gate review for G01 plus GS01-GS04 interface compatibility against the 001 T050 current-pin evidence and T021 deletion inventory; record the exact approve/reject decision and any remaining G02-G16c downlink exclusions in `crates/helix-im/docs/interaction-v2/09-全Gate覆盖矩阵与raw-emit下线清单.md`.
- [ ] T040 After T039 records an approval, physically remove the inventoried G01-only raw projection producer/consumer, fixture, subscription and documentation references; prove absence with targeted `rg` and update `crates/helix-im/src/send/flow.rs`, `crates/helix-im/tests/f5_golden_replay.rs`, and `CSES_CLIENT_WORKTREE/src/pages/message-v4/__tests__/surface-runtime-hard-cut.test.ts`.
- [ ] T041 Run the complete post-deletion Quickstart evidence matrix, Rust focused/broad regression, PC Jest/desktop DOM/hard-cut gates, and append command start/end/duration, pass/block, failed counterexample, SQLite/PG read results, and 30-minute progress records to `specs/002-ifv2-g01-online-send/quickstart.md`.

## Requirements Traceability

| Requirement | Tasks |
|---|---|
| FR-001, FR-002 | T012, T016, T025, T028 |
| FR-003, FR-014 | T012, T017-T019 |
| FR-004, FR-005, FR-006 | T013, T019-T021, T023, T040 |
| FR-007, FR-008 | T025-T031 |
| FR-009 | T032-T036 |
| FR-010, FR-012 | T037, T041 |
| FR-011, SC-004 | T005-T007, T012-T015, T025-T027, T032-T033, T041 |
| FR-013, SC-005 | T001, T024, T031, T036-T037, T041 |
| SC-001 | T012-T024, T040-T041 |
| SC-002 | T025-T031, T041 |
| SC-003 | T032-T037, T041 |

## Dependencies & Execution Order

1. T001 is a hard predecessor for T002-T041. A missing, stale-pin or non-PASS 001 T050 verdict blocks every implementation task; no fixture/source/worktree/test activity is allowed before it.
2. Phase 1 -> Phase 2 is mandatory: admitted fixture identifiers and evidence format precede G01 type/codec extension work.
3. Phase 2 blocks every story: the accepted 001 V2 typed contract and atomic Cache are prerequisites for any G01 producer or DOM claim.
4. US1 is the MVP. US2 depends on US1’s pending/canonical model but is independently verifiable through injected failure fixtures. US3 depends on US1’s canonical producer and can be developed while US2 is stabilised after Phase 2.
5. Phase 6 starts only after desired stories complete. T039 rechecks 001 T050 plus the prerequisite V2/27-Gate approval; T040 is the only physical deletion task and runs only after that approval; T041 supplies the final post-deletion evidence. A rejected T039 blocks T040-T041 from claiming a hard cut.

## Parallel Opportunities

- No code/source task is parallelisable before T001. After T001, T002/T003 and T005-T007 touch distinct G01 fixture/test cases and can proceed in parallel without changing the 001 shared schema.
- T012-T015 are independent test files; T025-T027 and T032-T033 are likewise parallel test preparation.
- T022 (G01 ingress integration) and T023 (composer/render) can parallelize once T009-T010 and the Helix frame shape are stable; neither may change generic 001 cache ownership.
- T036 (desktop chain) and T037 (bounded read-only reconciliation) can parallelize after a valid canonical run exists; neither may write data.

## Implementation Strategy

### MVP First

1. Run only T001 first. Stop immediately unless 001 T050 has a current-pin four-plane `PASS`; do not create an implementation worktree or run implementation commands before that.
2. Finish Phase 1 and Phase 2 as G01 extensions of the accepted baseline.
3. Execute US1 tests first, implement T016-T023 as one V2 producer/consumer cutover readiness increment, then run T024.
4. Stop if `Sending -> Sent` cannot be proven by a single row, authoritative confirmation, atomic frame cache, and a V2-only new route; do not delete legacy raw code before T039 approval.

### Incremental Delivery

1. After T001, US1 establishes truthful Sending/Sent and V2-only cutover readiness on the 001 baseline.
2. US2 makes every rejection/failure recoverable and fail-closed.
3. US3 adds cross-viewer isolation and full evidence reconciliation.
4. Phase 6 records T039 review approval, performs the one physical legacy deletion in T040, then uses T041 to confirm no regression and no G02-G16c overclaim.
