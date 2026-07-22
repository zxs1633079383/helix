# Tasks: SPEC-IFV2-GS01-GS04-同步恢复

**Input**: [spec.md](./spec.md), [plan.md](./plan.md), [research.md](./research.md), [data-model.md](./data-model.md), and [contracts](./contracts/).

**Tests are mandatory**: the Feature requires Rust deterministic tests, cses-client DOM tests, PC/mobile shared-frame parity, and readonly SQLite/PG evidence. Each implementation slice therefore starts with its falsifiable test/fixture task.

## Format And Boundaries

- `helix-im` owns recovery semantics, ledger specifications, composer and V2 Frame production.
- `helix-core` remains unchanged unless a truly generic port gap is demonstrated and separately approved.
- `helix-driver-host` executes generic transactional storage only; it must not interpret IM event semantics.
- PC/mobile only decode, stage, atomically swap, ack/resync and render the V2 contract. No task may reinstate a raw business-event reducer.
- `mcp__helix_local_sqlite_ro__*` and `mcp__cses_pre_postgres_ro__*` are validation-only. No task permits direct DB writes.

## Phase 1: Setup And Characterization

**Purpose**: Freeze the actual V1 baseline and the local database/schema owners before adding V2 behavior.

- [X] T001 Record current V1 eight-surface/Frame contract, `IM_SCHEMA` execution route, existing raw emit producers, and the expected ninth composer surface in `specs/001-gs-sync-recovery/research.md`.
- [X] T002 Record bounded readonly SQLite/PG query shapes, evidence-pack fields, and the 30-minute progress record in `specs/001-gs-sync-recovery/contracts/sync-recovery-evidence.md`.
- [X] T003 [P] Add GS01-GS04 V2 fixture naming, source pin and manifest rules in `crates/helix-im/tests/fixtures/uc01-16-surface-runtime/manifest.json`.
- [X] T004 [P] Add the matching PC fixture mirror manifest contract in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__fixtures__/uc01-16-surface-runtime/fixture-manifest.json`.
- [X] T005 [P] Record the mobile GS01-GS04 fixture migration boundary and source-pin check in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/fixtures/uc01-16/GS01-04/canonical_expected.json`.

## Phase 2: Foundational V2 Contract And Persistence

**Purpose**: Build the shared prerequisites before any recovery user story. No story starts before this phase completes.

- [X] T006 Add the ninth `message.composer` surface, typed `ComposerVm`, V2 runtime mutation, and strict v2 validation to `crates/helix-im/src/interaction_frame.rs`.
- [X] T007 Add ninth-surface, runtime, unique-slot, schema-v2, epoch/revision and ActionBinding fail-closed cases to `crates/helix-im/tests/interaction_frame_contract_test.rs`.
- [X] T008 [P] Extend `IM_SPARSE_TABLE_SPECS`, `IM_SCHEMA`, and additive migration literals for `channel_event_ledger` and `channel_sync_coverage` in `crates/helix-im/src/schema.rs`.
- [X] T009 [P] Add schema migration/idempotence and transaction rollback coverage for the new IM tables in `crates/helix-driver-host/tests/im_event_persist.rs`.
- [X] T010 Define O(1) ledger/coverage `StorageOp` builders with canonical event/fact hash inputs in `crates/helix-im/src/acl/storage_effects.rs`.
- [X] T011 Convert sync fact persistence to one correlated write-only `PersistAtomic` write set containing facts, ledger, coverage, and cursor in `crates/helix-im/src/sync/flow.rs`.
- [X] T012 Prove a failed atomic write rolls back facts, ledger, coverage and cursor without emitting a final frame in `crates/helix-im/tests/gs_persist_barrier_test.rs`.
- [ ] T013 Create the attach-aware `SurfaceChangeSet` and channel interaction-bundle interface in `crates/helix-im/src/surface_composer.rs`.
- [ ] T014 Add V2 GS01-GS04 canonical, duplicate, revision-gap, persist-failure and unknown-channel fixtures in `crates/helix-im/tests/fixtures/uc01-16-surface-runtime/`.
- [ ] T015 [P] Add `message.composer` scope, `ComposerVm`, runtime decoding and strict v2 validation in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts` and `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.codec.ts`.
- [ ] T016 Update FrameCache staging so runtime plus all attached V2 slots publish through one version swap and only post-commit directives/acks run in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts`.
- [ ] T017 [P] Add v2 composer/runtime and whole-frame atomicity tests to `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts`.
- [ ] T018 [P] Add `message.composer` V2 decode/stage/ack/resync support in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.hpp` and `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp`.
- [ ] T019 [P] Add the same GS01-GS04 V2 fixture apply/revision-gap tests in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/DialogTimelineUnreadParityTest.cpp`.

**Checkpoint**: V2 can represent all nine surfaces, SQLite can atomically retain an auditable event range, and PC/mobile reject a bad Frame without a partial cache publication.

## Phase 3: User Story 1 - Reconnect Restores Authority (Priority: P1)

**Goal**: A reconnect compares committed local and remote authority sequences, restores missing facts atomically, and renders global sync state plus committed dialog/unread state.

**Independent test**: Seed a local committed cursor below a fixture authority head, drive reconnect/hello through Tick, then verify ledger/coverage/cursor, V2 Frame and PC DOM. Repeat with equal sequences and injected persistence failure.

- [ ] T020 [US1] Add a deterministic reconnect/equal-sequence/persist-failure replay harness in `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`.
- [ ] T021 [US1] Model recovery phase, authority head and per-channel committed comparison inputs in `crates/helix-im/src/sync/session.rs`.
- [ ] T022 [US1] Route `Tick::Connected`, hello and reconnect observations into a new recovery session without declaring UI completion in `crates/helix-im/src/module/trait_impl.rs` and `crates/helix-im/src/sync/scheduler.rs`.
- [ ] T023 [US1] Implement local-cursor versus authority-head comparison, equal no-op, local-behind pull and local-ahead/invalid-gap escalation in `crates/helix-im/src/sync/explicit.rs` and `crates/helix-im/src/sync/flow.rs`.
- [ ] T024 [US1] Normalize initial increment and sync-notify replies into validated `SyncBatchFacts` with contiguous event evidence in `crates/helix-im/src/sync/increment.rs` and `crates/helix-im/src/channel/gate/flow.rs`.
- [ ] T025 [US1] Persist the committed recovery batch and advance only in-memory sequence state from its matching `PersistOk` reply in `crates/helix-im/src/channel/gate/commit.rs` and `crates/helix-im/src/sync/flow.rs`.
- [ ] T026 [US1] Compose `runtime(syncing/recovered) + left-dialog-list + unread-summary` as one post-commit V2 Frame in `crates/helix-im/src/surface_composer.rs` and `crates/helix-im/src/interaction_frame.rs`.
- [ ] T027 [US1] Wire the V2 frame batch and FrameApplied/SurfaceResync requests through the PC Helix bridge in `/System/Volumes/Data/workspace/angular/cses-client/src-tauri/src/features/helix_im/runtime.rs`.
- [ ] T028 [US1] Render and clear the global synchronising state solely from the V2 runtime mutation in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts`.
- [ ] T029 [US1] Add a TestBed DOM scenario for global syncing, atomically recovered dialog/unread content, equal-sequence no duplication and persist-failure no-final-Frame in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts`.
- [ ] T030 [US1] Add a correlated authority-head/ledger/coverage/cursor assertion over recovery fixtures in `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`.

**Checkpoint**: US1 is independently demoable when the Rust replay and PC DOM test pass; no direct database inspection is needed to make the UI look recovered.

## Phase 4: User Story 2 - Relogin Has a New Trust Boundary (Priority: P1)

**Goal**: An A-to-B relogin discards A's in-flight recovery, surface epochs and action references; B compares and presents only B's committed scope.

**Independent test**: Drive two identities through the same host process and confirm B cannot invoke/see A's bindings, cached slots, ledger coverage or recovery result.

- [ ] T031 [US2] Add a deterministic A-to-B relogin, stale PortReply and stale ActionBinding rejection fixture to `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`.
- [ ] T032 [US2] Invalidate recovery correlation, per-channel state and action capability epochs on actor/session change in `crates/helix-im/src/state.rs` and `crates/helix-im/src/module/action.rs`.
- [ ] T033 [US2] Scope ledger/coverage writes and cursor scans to the active tenant/actor boundary in `crates/helix-im/src/schema.rs` and `crates/helix-im/src/acl/storage_effects.rs`.
- [ ] T034 [US2] Emit a new-session runtime/frame baseline and reject stale session effects before composition in `crates/helix-im/src/sync/session.rs` and `crates/helix-im/src/surface_composer.rs`.
- [ ] T035 [US2] Add session reset that detaches/invalidate old cache slots and pending action refs without locally inventing business state in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-runtime.service.ts`.
- [ ] T036 [US2] Add a TestBed DOM assertion that actor B sees only B's post-relogin runtime, slots and enabled bindings in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts`.

**Checkpoint**: US2 passes when any late A effect/frame/receipt is fail-closed and B recovery still completes normally.

## Phase 5: User Story 3 - Unknown Channel Hydrates Before Send (Priority: P2)

**Goal**: A route or sync event for an unknown channel cannot enable sending until channel/member/message facts commit; summary and composer appear atomically afterward.

**Independent test**: Attach an unknown channel, inject a qualifying sync event, and assert absent/disabled send before `PersistOk`, then a shared summary+composer frame with enabled action after it.

- [ ] T037 [US3] Add unknown-channel hydration, rollback and summary+composer atomic bundle cases to `crates/helix-im/tests/gs01_04_v2_sync_recovery_test.rs`.
- [ ] T038 [US3] Turn unknown-channel inbound/route observations into scoped hydration recovery rather than direct projection in `crates/helix-im/src/channel/gate/flow.rs` and `crates/helix-im/src/sync/scheduler.rs`.
- [ ] T039 [US3] Include required channel/member/viewer/message facts and ledger coverage in unknown-channel `PersistAtomic` construction in `crates/helix-im/src/sync/flow.rs` and `crates/helix-im/src/acl/storage_effects.rs`.
- [ ] T040 [US3] Compose a single `message.channel-summary + message.composer + runtime` interaction bundle with an enabled send binding only after hydration commit in `crates/helix-im/src/surface_composer.rs`.
- [ ] T041 [US3] Consume the composer Surface and invoke its opaque action only through `SurfaceRuntime` context in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/message-surface.facade.ts` and `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/actions/surface-action-invocation.ts`.
- [ ] T042 [US3] Add a TestBed DOM assertion for pre-commit no-send and post-commit shared summary/composer visibility in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts`.
- [ ] T043 [US3] Add the unknown-channel composer V2 fixture to mobile apply tests in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/SurfaceRuntimeTest.cpp`.

**Checkpoint**: US3 passes when no renderer can construct a send request for a channel without committed hydration, while a committed bundle enables normal sending.

## Phase 6: Cross-Cutting Acceptance And Cutover Readiness

**Purpose**: produce credible evidence and prove that V2 is ready for the separately authorised physical raw-producer deletion.

- [ ] T044 [P] Add readonly SQLite/PG comparison helpers and bounded query documentation for message/channel/member/event-ledger/coverage/cursor ranges in `specs/001-gs-sync-recovery/quickstart.md`.
- [ ] T045 [P] Update the shared fixture parity test to require the new GS01-GS04 V2 files and hashes in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.fixture-parity.test.ts`.
- [ ] T046 [P] Update mobile source-pin and V2 fixture parity checks in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/check-surface-runtime-source-pin.mjs` and `/System/Volumes/Data/workspace/c/mobile-qucik-c++/flutter_plugin/test/surface_runtime_apply_test.dart`.
- [ ] T047 Add hop-complete correlation diagnostics from compare through FrameApplied/ack without exposing VM contents in `crates/helix-im/src/sync/flow.rs` and `crates/helix-driver-host/src/engine.rs`.
- [ ] T048 Add a no-raw-fallback V2 consumer gate and a raw-producer deletion readiness inventory in `crates/helix-im/tests/interaction_frame_contract_test.rs` and `crates/helix-im/docs/interaction-v2/09-全Gate覆盖矩阵与raw-emit下线清单.md`.
- [ ] T049 Run focused Rust, PC DOM, mobile fixture, readonly SQLite/PG and correlation-evidence checks using `specs/001-gs-sync-recovery/quickstart.md`; record actual command start/end/wall time and `PASS|FAIL|BLOCKED` in `specs/001-gs-sync-recovery/contracts/sync-recovery-evidence.md`.
- [ ] T050 Update the GS01-GS04 completion/cutover decision, remaining raw producer owners and any human escalation in `specs/001-gs-sync-recovery/contracts/sync-recovery-evidence.md`.

## Dependencies And Execution Order

```text
T001-T005 -> T006-T019
T006-T019 -> US1 (T020-T030)
T006-T019 -> US2 (T031-T036)
T006-T019 -> US3 (T037-T043)
US1 + US2 + US3 -> T044-T050
```

- US1 is the MVP and must land first because it proves the recovery/persist/frame spine.
- US2 and US3 may proceed in parallel after Phase 2, provided each owner does not touch the same files without coordination.
- T044-T046 can run in parallel after fixture shapes stabilise; T047-T050 follow the integrated behavior.

## Parallel Opportunities

```text
Foundation: T008 + T015 + T018 can run in parallel after T006 contract names settle.
US1: T027 and T028 can run in parallel once T026 emits final frame shapes.
US2: T032 and T035 can run in parallel after T031 locks stale-session assertions.
US3: T038 and T041 can run in parallel after T037 locks hydration semantics.
Acceptance: T044 + T045 + T046 can run in parallel after all fixture files exist.
```

## Implementation Strategy

1. Complete Phase 1 and Phase 2; stop if the schema ownership or V2 fixture contract differs from source truth.
2. Implement and independently verify US1. This is the smallest usable recovery slice.
3. Add US2 and US3, each with its own falsifiable test before integration.
4. Assemble four-plane evidence and hold the physical raw-producer deletion until the V2 docs and acceptance evidence pass review.

## Completion Criteria

- All 50 tasks are completed with no skipped mandated test/evidence task.
- Each user story has a passing independent test, while final sign-off also has integrated evidence.
- `PASS` is never inferred from a plan, an HTTP 2xx, a connected socket, a cursor alone, or a manually changed database.
- The raw emit producer deletion remains a separate authorised change after this Feature's readiness decision.
