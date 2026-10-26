# Tasks: Timeline V3 Delta Window

**Input**: Design documents from `specs/046-timeline-v3-delta-window/`

**Tests**: Producer, canonical fixture, PC WDIO and Mobile GTest work is explicitly required.
Test tasks precede implementation tasks and must first falsify the current V2/partial-V3 behavior.

## Phase 1: Setup

- [X] T001 Record Helix/cses-client/Mobile HEAD, pin and dirty-path exclusions in `specs/046-timeline-v3-delta-window/evidence/source-lock.txt`
- [X] T002 Inventory the 13 existing Timeline Gate producer/fixture/PC/Mobile assets in `specs/046-timeline-v3-delta-window/evidence/gate-inventory.txt`
- [X] T003 [P] Freeze the four formal Gate file inventories in `crates/helix-im/tests/fixtures/README.md`
- [X] T004 [P] Update the copied product contract and Spec Kit links in `docs/2026-07-27-message-v4-timeline-v3-prd.md`

## Phase 2: Foundational

- [X] T005 Add failing schema/operation/stable-key contract cases in `crates/helix-im/tests/timeline_v3_contract_test.rs`
- [X] T006 [P] Add failing default/20/60/0/61 and locate-allocation tests in `crates/helix-im/tests/timeline_window_policy_test.rs`
- [X] T007 Complete closed Timeline V3 Snapshot/Delta validation and stable-key invariants in `crates/helix-im/src/interaction_frame.rs`
- [X] T008 Implement shared bounded page/locate allocation policy in `crates/helix-im/src/projector/timeline.rs`
- [X] T009 Migrate the 13 existing Timeline Gate producers and positive canonical assets under `crates/helix-im/tests/` from Replace/extra Activate to exact V3 operations
- [X] T010 Extend canonical semantic hashing/provenance validation in `crates/helix-im/tests/support/timeline_formal_gate.rs`
- [X] T011 Bump Strict Matrix to epoch 12 and declare G-10b/G-10c/G-10d/G-11a in `crates/helix-im/contracts/strict-matrix.v1.json`

**Checkpoint**: Helix shared V3 contract, page policy and existing Gate assets are authoritative. No
PC or Mobile fixture may be authored before T009-T011 pass.

## Phase 3: User Story 1 - Initial latest window (Priority: P1)

**Independent Test**: latest with missing/20/60 returns an authoritative Snapshot of at most N
messages and correct lookahead bounds; 0/61 performs no I/O.

- [X] T012 [P] [US1] Add latest Snapshot producer tests in `crates/helix-im/tests/timeline_latest_window_test.rs`
- [X] T013 [US1] Implement `pageSize+1` latest request, correlation and persisted bounds in `crates/helix-im/src/outbound/posts/read.rs`, `crates/helix-im/src/port_reply/` and `crates/helix-im/src/timeline_navigation.rs`
- [X] T014 [US1] Compose bounded latest Snapshot from committed facts in `crates/helix-im/src/projector/timeline.rs`
- [X] T015 [US1] Add PC initial V3 decode/apply tests in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts`
- [X] T016 [US1] Add Mobile initial V3 Snapshot cases in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/SurfaceRuntimeTest.cpp`

## Phase 4: User Story 2 - Bounded live updates (Priority: P1)

**Independent Test**: send/sent/failed/revoke/reaction/urgent and live append use one stable entity,
no Replace and no implicit activation; anchored consumers keep their window.

- [X] T017 [P] [US2] Add ordinary-update and full-window append producer tests in `crates/helix-im/tests/timeline_delta_update_test.rs`
- [X] T018 [US2] Make send/update/revoke projection emit only stable Upsert and new-key edge Insert in `crates/helix-im/src/projector/timeline.rs`
- [X] T019 [US2] Generate G-10c/G-10d through the shared production ScenarioRunner in `crates/helix-im/tests/support/timeline_formal_gate.rs`
- [X] T020 [US2] Implement PC FollowLatest/Anchored/pending-hint/jump state in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/`
- [X] T021 [US2] Implement Mobile FollowLatest/Anchored/pending-hint/jump state in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/SurfaceRuntime.cpp`

## Phase 5: User Story 3 - Bidirectional paging (Priority: P1)

**Independent Test**: older and newer pages at 20/60 extend only their Range edge, update proved
bounds, preserve anchor, and stop after exhaustion.

- [X] T022 [P] [US3] Add G-10a/G-10b directional page producer tests in `crates/helix-im/tests/g10b_wdio_fixture_contract_test.rs`
- [X] T023 [US3] Add newer-direction request and correlated reply handling in `crates/helix-im/src/outbound/posts/read.rs`, `crates/helix-im/src/port_reply/` and `crates/helix-im/src/timeline_navigation.rs`
- [X] T024 [US3] Project directional Upsert/Insert/Bounds without implicit Activate in `crates/helix-im/src/projector/timeline.rs`
- [X] T025 [US3] Generate the G-10b canonical inventory under `crates/helix-im/tests/fixtures/g10b-wdio/`
- [X] T026 [US3] Implement PC loadNewer action, Range apply and G-10b WDIO assertions in `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/`
- [X] T027 [US3] Implement Mobile newer-page apply and G-10b GTest in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/G10BFrameContractTest.cpp`

## Phase 6: User Story 4 - Locate and navigate both directions (Priority: P1)

**Independent Test**: uncached target is at the deterministic center when possible, short-side
quota refills, current token activates, stale token does not, and both directions remain pageable.

- [X] T028 [P] [US4] Add G-11a locate/balance/token producer tests in `crates/helix-im/tests/g11a_wdio_fixture_contract_test.rs`
- [X] T029 [US4] Implement bounded target-context request orchestration in `crates/helix-im/src/outbound/posts/read.rs`, `crates/helix-im/src/port_reply/` and `crates/helix-im/src/timeline_navigation.rs`
- [X] T030 [US4] Project InstallSegment/current-token Activate and stale cache-only results in `crates/helix-im/src/projector/timeline.rs`
- [X] T031 [US4] Generate the G-11a canonical inventory under `crates/helix-im/tests/fixtures/g11a-wdio/`
- [X] T032 [US4] Implement PC locate/dual-page navigation and G-11a WDIO in `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/`
- [X] T033 [US4] Implement Mobile locate/dual-page navigation GTest in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/G11AFrameContractTest.cpp`

## Phase 7: User Story 6 - Gap recovery (Priority: P1)

**Independent Test**: invalid epoch/revision/Range/Bounds/mixed transaction changes no visible state,
coalesces recovery and resumes only after Snapshot.

- [X] T034 [P] [US6] Add invalid-delta and recovery producer/consumer fixtures in `crates/helix-im/tests/fixtures/timeline-v3-recovery/`
- [X] T035 [US6] Complete atomic shadow validation and recovery Snapshot composition in `crates/helix-im/src/interaction_frame.rs` and `crates/helix-im/src/projector/timeline.rs`
- [X] T036 [US6] Add PC atomic reject/recovery tests in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/surface-runtime.service.test.ts`
- [X] T037 [US6] Add Mobile atomic reject/recovery tests in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/SurfaceRuntimeTest.cpp`

## Phase 8: User Story 7 - Timeline V2 hard cut (Priority: P1)

**Independent Test**: any positive Timeline V2 Frame fails, all positive Timeline paths are V3, and
unaffected non-Timeline V2 assets remain accepted.

- [X] T038 [P] [US7] Add producer and consumer hard-cut negative tests in `crates/helix-im/tests/timeline_v3_hard_cut_test.rs`
- [X] T039 [US7] Remove Timeline V2 generation and compatibility branches from `crates/helix-im/src/` after positive Helix Gates pass
- [X] T040 [US7] Remove Timeline V2 decode/fixture paths from `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/` after PC V3 Gates pass
- [X] T041 [US7] Remove Timeline V2 decode/fixture paths from `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/` and `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/` after Mobile V3 Gates pass

## Phase 9: User Story 8 - Producer-owned cross-client Gate assets (Priority: P1)

**Independent Test**: all declared Helix assets validate, PC/Mobile bytes match, operation-level
normalized results match, and four formal Gates appear in each report.

- [X] T042 [P] [US8] Add G-10c/G-10d fixture contract tests in `crates/helix-im/tests/g10c_wdio_fixture_contract_test.rs` and `crates/helix-im/tests/g10d_wdio_fixture_contract_test.rs`
- [X] T043 [US8] Generate G-10c/G-10d canonical inventories and provenance under `crates/helix-im/tests/fixtures/g10c-wdio/` and `crates/helix-im/tests/fixtures/g10d-wdio/`
- [X] T044 [US8] Upgrade PC fixture loader/sync/hash/provenance tools in `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/fixtures/` and `/System/Volumes/Data/workspace/angular/cses-client/scripts/message-wdio/`
- [X] T045 [US8] Add PC G-10c/G-10d PageObjects, harnesses, reporters and WDIO mock/real contracts in `/System/Volumes/Data/workspace/angular/cses-client/tests/e2e/wdio/`
- [X] T046 [US8] Upgrade Mobile fixture sync/hash/provenance tools in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/fixtures/` and `/System/Volumes/Data/workspace/c/mobile-qucik-c++/scripts/`
- [X] T047 [US8] Add Mobile G-10c/G-10d test classes in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/G10CFrameContractTest.cpp` and `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/G10DFrameContractTest.cpp`
- [X] T048 [US8] Run and record byte/semantic parity for all affected Gates in `specs/046-timeline-v3-delta-window/evidence/cross-client-parity.txt`

## Phase 10: User Story 5 - Bounded multi-channel cache (Priority: P2)

**Independent Test**: deterministic small budgets evict unpinned chunks while preserving current,
anchor, pending and in-flight data; cold re-entry rebuilds from persistence.

- [X] T049 [P] [US5] Add cache budget/pin/cold-channel tests in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/__tests__/timeline-v3-cache.test.ts`
- [X] T050 [P] [US5] Add equivalent Mobile cache budget/pin tests in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/tests/gtest/surface_runtime/TimelineCacheTest.cpp`
- [X] T051 [US5] Implement indexed chunk cache and pin release rules in `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/timeline-v3-cache.ts`
- [X] T052 [US5] Implement equivalent indexed chunk cache in `/System/Volumes/Data/workspace/c/mobile-qucik-c++/cpp/TimelineCache.cpp`

## Phase 11: Polish and evidence

- [X] T053 Update Strict Matrix generated reports and Gate documentation in `crates/helix-im/docs/Strict Matrix/`
- [X] T054 Run focused Helix, PC and Mobile validators from `specs/046-timeline-v3-delta-window/quickstart.md` and save bounded outputs under `specs/046-timeline-v3-delta-window/evidence/`
- [X] T055 Run eligible PC/Mobile real lanes without substituting Mock for missing prerequisites and record blockers/results under `specs/046-timeline-v3-delta-window/evidence/`
- [X] T056 Run four-repository `git diff --check`, dirty-path audit and Timeline V2 hard-cut scan into `specs/046-timeline-v3-delta-window/evidence/final-static.txt`
- [X] T057 Register verification outputs in `specs/046-timeline-v3-delta-window/verification.md`, reconcile `SESSION.md`, and run the mandatory R4 loop gate

## Phase 12: Blocked producer convergence

**Purpose**: The user authorized cses-im-server as a fourth implementation repository on
2026-07-28. Current projector fixtures remain L0 until these tasks pass.

- [X] T058 [US3] Add same-timestamp composite newer-cursor contract tests in `/System/Volumes/Data/workspace/golang/cses-im-server/internal/store/sqlstore/cses_post_timeline_test.go`
- [X] T059 [US3] Implement stable `(createAt, temporaryId)` latest/older/newer/locate filter/order and bounded page contract in `/System/Volumes/Data/workspace/golang/cses-im-server/internal/store/sqlstore/cses_post_context.go`, App and API wire
- [X] T060 [US3] Implement Helix production `im_load_newer_context` and uncached locate command/correlation/Persist/readback ScenarioRunners in `crates/helix-im/src/` and `crates/helix-im/tests/support/`
- [X] T061 [US8] Regenerate G-10b/G-10c/G-10d/G-11a through `ImModule::handle -> Effect::Emit`, promote provenance to canonical, then release PC/Mobile fixture sync in `crates/helix-im/tests/fixtures/`

## Dependencies and execution order

- T001-T011 are blocking foundations.
- All Helix producer tasks T012-T014, T017-T019, T022-T025, T028-T031, T034-T035,
  T038-T039 and T042-T043 must pass before any PC canonical sync task T044 or Mobile sync task T046.
- PC consumer tasks T015, T020, T026, T032, T036, T040, T044-T045 and T049/T051 depend on the
  corresponding Helix producer assets.
- Mobile tasks T016, T021, T027, T033, T037, T041, T046-T047 and T050/T052 start only after the
  corresponding Helix asset is Green; byte parity never depends on a hand-authored consumer file.
- T039-T041 physical hard-cut deletion occurs only after each repository's positive V3 Gates pass.
- T048 and T053-T057 depend on all selected delivery phases.
- T058-T061 are now admitted in order; T060-T061 then unblock T026-T027, T032-T033 and T044-T048.

## Phase 13: Convergence

- [x] T062 Commit the verified Helix production producer foundation, regenerate formal fixture provenance against that immutable 40-character revision, and record the new pin per SC-014
- [ ] T063 Repin and exact-sync PC/Mobile to the immutable Helix producer, rebuild the PC WDIO candidate, then rerun the eligible native WDIO lane and Mobile immutable source proof per SC-014 and SC-022 (partial)

## Parallel opportunities

- T003/T004, T005/T006, and PC/Mobile test authoring on distinct files may run in parallel after
  their producer contracts are fixed.
- G-10b, G-10c/G-10d and G-11a Helix scenario files have distinct ownership but converge through
  shared projector/repository work, so shared files remain serialized.
- PC and Mobile consumer implementation can run in parallel only after the complete Helix canonical
  inventory is frozen.

## Implementation strategy

The first independently useful increment is the Helix foundation plus US1: validated schema,
page policy and bounded latest Snapshot. Delivery then adds live Delta, directional pages, locate,
recovery and hard-cut. Cross-client synchronization is deliberately last in each producer slice.
Each Green Helix/PC/Mobile batch is commit-eligible only when it excludes unrelated dirty files.

## Loop Task Contracts

| Task | Requirement | Plan Item | Change Scope | Preconditions | Validator | Done Signal |
|---|---|---|---|---|---|---|
| T001 | FR-038 | I-005 | source lock evidence | R2 PASS | V-009 | three HEAD/pins and exclusions recorded |
| T002 | FR-036 | I-005 | Gate inventory evidence | T001 | V-003 | 13 existing families mapped |
| T003 | FR-058 | A-008 | Helix fixture README | T002 | V-003 | four inventories fixed |
| T004 | FR-047 | A-008 | copied PRD | R2 PASS | V-001 | Helix spec/Gate links current |
| T005 | FR-002,FR-040 | S-001 | Rust contract test | T001 | V-002 | current partial contract falsified |
| T006 | FR-048,FR-051 | S-005 | Rust policy test | T001 | V-002 | 20/60/0/61 and center covered |
| T007 | FR-002,FR-028 | A-007 | InteractionFrame types | T005 | V-002 | closed atomic V3 validation passes |
| T008 | FR-048,FR-052 | A-001,A-003 | Helix window policy | T006 | V-002 | bounded allocation passes |
| T009 | FR-041,FR-053 | A-004 | existing Helix Gates | T007-T008 | V-002,V-003 | 13 families have exact V3 ops |
| T010 | FR-038,FR-062 | I-005 | fixture hash/provenance | T003 | V-003 | semantic drift is detected |
| T011 | FR-058,SC-024 | S-006 | Strict Matrix JSON | T003 | V-007 | epoch12 50/47/3 validates |
| T012 | FR-006,FR-050 | A-001 | latest producer test | T007-T008 | V-002 | bounded Snapshot test fails then passes |
| T013 | FR-013,FR-050 | A-001 | Helix repository | T012 | V-002 | N+1 committed bounds returned |
| T014 | FR-007 | A-001 | Helix projector | T013 | V-002 | latest Snapshot at most N |
| T015 | FR-044 | S-001 | PC runtime test | T014 | V-004 | PC applies initial V3 atomically |
| T016 | FR-045 | S-001 | Mobile runtime test | T014 | V-005 | Mobile applies initial V3 atomically |
| T017 | FR-008,FR-041 | A-004,A-006 | update producer test | T009 | V-002 | Replace/Activate regression fails |
| T018 | FR-041,FR-063 | A-004 | Helix projector | T017 | V-002 | one stable entity per update |
| T019 | FR-055,FR-056 | A-005,A-006 | G10c/G10d Helix scenarios | T018 | V-002,V-003 | producer classes pass |
| T020 | FR-056 | S-002 | PC selector/runtime | T019 | V-004 | anchor/hint/jump pass |
| T021 | FR-061 | S-002 | Mobile selector/runtime | T019 | V-005 | equivalent anchor/hint/jump pass |
| T022 | FR-043,FR-054 | A-002 | G10b producer test | T008-T009 | V-002 | newer direction falsifiable |
| T023 | FR-013,FR-054 | A-002 | Helix repository/handler | T022 | V-002 | newer correlated reply works |
| T024 | FR-014,FR-054 | A-002 | Helix projector | T023 | V-002 | edge/range/bounds exact |
| T025 | FR-058 | A-008 | G10b canonical assets | T024 | V-003 | declared inventory validates |
| T026 | FR-060 | A-002 | PC G10b WDIO | T025,T044 | V-004 | loadNewer DOM/state assertions pass |
| T027 | FR-061 | A-002 | Mobile G10b GTest | T025,T046 | V-005 | newer state assertions pass |
| T028 | FR-021,FR-057 | A-003 | G11a producer test | T008-T009 | V-002 | balance/token cases falsifiable |
| T029 | FR-051,FR-052 | A-003 | Helix repository/handler | T028 | V-002 | target context bounded |
| T030 | FR-021,FR-057 | A-003 | Helix projector | T029 | V-002 | segment/token semantics pass |
| T031 | FR-058 | A-008 | G11a canonical assets | T030 | V-003 | five declared files validate |
| T032 | FR-060 | S-003 | PC G11a WDIO | T031,T044 | V-004 | centered dual-page behavior passes |
| T033 | FR-061 | S-003 | Mobile G11a GTest | T031,T046 | V-005 | equivalent locate behavior passes |
| T034 | FR-028,FR-029 | A-007 | recovery fixtures | T007 | V-002 | invalid transactions covered |
| T035 | FR-030,FR-031 | A-007 | Helix recovery path | T034 | V-002 | last-good/recovery passes |
| T036 | FR-044 | S-001 | PC recovery test | T035 | V-004 | no partial PC commit |
| T037 | FR-045 | S-001 | Mobile recovery test | T035 | V-005 | no partial Mobile commit |
| T038 | FR-034,FR-046 | A-008 | hard-cut tests | T009 | V-008 | positive Timeline V2 rejected |
| T039 | FR-035 | A-008 | Helix V2 removal | T038 and Helix V3 Green | V-008 | producer compatibility count zero |
| T040 | FR-035 | A-008 | PC V2 removal | PC V3 Green | V-008 | PC compatibility count zero |
| T041 | FR-035 | A-008 | Mobile V2 removal | Mobile V3 Green | V-008 | Mobile compatibility count zero |
| T042 | FR-055,FR-056 | A-008 | G10c/G10d contract tests | T019 | V-002 | inventories generated by producer |
| T043 | FR-038,FR-058 | A-008 | G10c/G10d canonical assets | T042 | V-003 | raw/semantic provenance passes |
| T044 | FR-039,FR-062 | A-008 | PC sync/hash tooling | all Helix assets Green | V-003,V-006 | PC bytes and hashes match |
| T045 | FR-060 | S-002,S-006 | PC WDIO Gate families | T020,T043-T044 | V-004 | G10c/G10d reports pass |
| T046 | FR-039,FR-062 | A-008 | Mobile sync/hash tooling | all Helix assets Green | V-003,V-006 | Mobile bytes and hashes match |
| T047 | FR-061 | S-002,S-006 | Mobile GTest classes | T021,T043,T046 | V-005 | G10c/G10d reports pass |
| T048 | FR-033,FR-040 | A-008 | parity evidence | T025-T027,T031-T033,T043-T047 | V-006 | byte and normalized parity pass |
| T049 | FR-023,FR-024 | S-001 | PC cache test | PC V3 runtime | V-004 | budget/pin cases falsifiable |
| T050 | FR-023,FR-024 | S-001 | Mobile cache test | Mobile V3 runtime | V-005 | equivalent cases falsifiable |
| T051 | FR-022-FR-027 | A-006 | PC TimelineCache | T049 | V-004 | indexed bounded cache passes |
| T052 | FR-022-FR-027 | A-006 | Mobile TimelineCache | T050 | V-005 | equivalent bounded cache passes |
| T053 | SC-024 | S-006 | Matrix reports/docs | T011,T048 | V-007 | reports expose 50/47/3 |
| T054 | FR-047 | A-008 | focused evidence | all selected tasks | V-002-V-006 | bounded outputs saved |
| T055 | FR-047 | A-008 | real-lane evidence | real prerequisites | V-004,V-005 | actual result/blocker recorded |
| T056 | FR-046 | A-008 | final static evidence | T039-T041 | V-008,V-009 | hard-cut/diff audit passes |
| T057 | FR-047 | S-006 | verification/SESSION/ledger | T053-T056 | V-001-V-009 | R4 verdict truthfully recorded |
| T058 | FR-050,FR-054 | A-002 | Go composite cursor tests | fourth-repo approval recorded | V-002 | same-timestamp loss is falsifiable |
| T059 | FR-050,FR-054 | A-002 | Go store/API contract | T058 | V-002 | 20/60 stable newer pages pass |
| T060 | FR-054,FR-057 | A-002,A-003 | Helix production scenarios | T059 | V-002,V-003 | command/persist/readback Frames pass |
| T061 | FR-037-FR-040 | A-008 | canonical promotion/sync release | T060 | V-003,V-006 | provenance canonicalEligible true |
| T062 | SC-014 | A-008 | immutable Helix producer provenance | T061,R4 PASS | V-003,V-006 | four formal inventories name a 40-character producer revision |
| T063 | SC-014,SC-022 | S-006 | immutable consumer repin and eligible real lanes | T062 | V-004-V-006 | PC candidate matches Helix and Mobile source proof is not skipped |
