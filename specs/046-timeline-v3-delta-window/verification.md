# Verification Procedure

## Required Runs

- **V-001 Spec admission**

  ```bash
  python3 .specify/extensions/helix-loop/scripts/loop_engine.py advance
  ```

  Expected before implementation: `IMPLEMENTATION_ADMITTED`; after evidence registration only
  `CLOSED` is completion.

- **V-002 Helix producer contracts**

  ```bash
  cargo check -p helix-im
  cargo test -p helix-im --test timeline_v3_contract_test
  cargo test -p helix-im --test timeline_window_policy_test
  cargo test -p helix-im --test g10b_wdio_fixture_contract_test
  cargo test -p helix-im --test g10c_wdio_fixture_contract_test
  cargo test -p helix-im --test g10d_wdio_fixture_contract_test
  cargo test -p helix-im --test g11a_wdio_fixture_contract_test
  ```

  Expected: schema/operation tests pass; `pageSize=20/60` passes; 0/61 fails before I/O; all
  formal Gate fixtures match their producer scenarios.

- **V-003 Canonical inventory and provenance**

  Run the repository fixture inventory/provenance validators over all 13 existing Timeline Gate
  families plus G-10b/G-10c/G-10d/G-11a.

  Expected: every `.frame-v3.json` is declared and raw/semantic SHA-256 matches; undeclared,
  hand-edited or operation-insensitive assets fail.

- **V-004 PC consumer**

  ```bash
  cd /System/Volumes/Data/workspace/angular/cses-client
  node scripts/message-wdio/sync-helix-fixtures.mjs --check
  npx jest -c tests/e2e/wdio/jest.config.js --runInBand
  npm run message:wdio:contracts
  node scripts/message-wdio/check-production-boundary.mjs
  ```

  Expected: V3 codec/cache/range/selector tests and four formal Gate WDIO contracts pass. Mock
  output is recorded as Mock only.

- **V-005 Mobile consumer**

  ```bash
  cd /System/Volumes/Data/workspace/c/mobile-qucik-c++
  scripts/build-helix-ffi.sh
  ./scripts/run-gtest-js-spec.sh surface-runtime \
    --gtest-filter='G10BFrameContractTest.*:G10CFrameContractTest.*:G10DFrameContractTest.*:G11AFrameContractTest.*'
  ```

  Expected: fixture provenance and V3 normalized/window behavior match PC for every formal Gate.

- **V-006 Cross-client parity**

  Run the implemented byte parity and semantic parity scripts in PC and Mobile.

  Expected: Helix/PC/Mobile raw bytes match and normalized operation/key/range/bounds/watermark/
  revision/window hashes match for every declared asset.

- **V-007 Strict Matrix**

  Run the Strict Matrix schema/generation/report validators under
  `crates/helix-im/docs/Strict Matrix/`.

  Expected: epoch 12, 50 total cases, 47 active, three excluded, and all four formal Gate IDs
  present in machine reports.

- **V-008 Timeline hard cut**

  Run the Helix, PC and Mobile positive-path scans/tests for Timeline V2 Replace.

  Expected: Timeline `timelinePatch + replaceView` producer/consumer count is zero; unaffected
  non-Timeline V2 fixtures remain accepted.

- **V-009 Static and dirty-tree safety**

  ```bash
  git diff --check
  git -C /System/Volumes/Data/workspace/angular/cses-client diff --check
  git -C /System/Volumes/Data/workspace/c/mobile-qucik-c++ diff --check
  ```

  Expected: no whitespace errors and no unrelated pre-existing dirty file is staged or reverted.

## Evidence Rules

Store raw command output beneath `evidence/`. Register each run with
`loop_engine.py record-evidence`; do not replace a failed output with prose.

Each record names actual start/end time, exit code, source revision and output file. Fixture/Mock,
static, persistence, WS, native/FFI and rendered evidence remain separate. A missing prerequisite is
`blocked`, not a reason to substitute another evidence plane.

## 2026-07-28 Implementation Results

| Evidence | Result | Boundary |
|---|---|---|
| V-001 admission | PASS through R3 | R4 rerun remains required after evidence registration |
| V-002 Helix | PASS | Contract 4/4, policy 5/5, update 3/3, latest 2/2, recovery 1/1, four formal Gates 4/4 |
| V-003 canonical inventory | PASS on current worktree | Four inventories contain 5/2/2/5 Frames and matching provenance |
| V-004 PC focused | PASS | 14 WDIO contracts plus 33 component assertions; native browser lane is blocked before startup |
| V-005 Mobile focused | PASS with one source-proof skip | Four formal classes 8/8; focused total 49 pass, 1 skip, 0 fail |
| V-006 parity | PASS on current worktree | Helix/PC/Mobile raw and semantic parity; immutable producer pin unresolved |
| V-007 Strict Matrix | PASS | Epoch 12 generated files and 9/9 contract tests pass |
| V-008 hard cut | PASS | Helix rejects positive Timeline V2; PC/Mobile positive Timeline V2 decoders and fixtures are physically removed |
| V-009 static | PASS | Four repositories pass `git diff --check`; unrelated dirty paths preserved |
| Go source/unit/API | PASS | Focused tests, build and vet pass |
| Go real PostgreSQL | BLOCKED | No `CSES_IM_TEST_DSN`; DDL/index validity and EXPLAIN are not proven |

The first final consumer run correctly found a producer bug: G-10b changed RangeId while keeping
the same ordered keys, but omitted `InstallSegment`. `delta_between` now emits `InstallSegment`
whenever RangeId changes and the producer regression assertion prevents recurrence. Both consumers
were resynchronized only after the corrected Helix fixtures passed.

All implementation tasks are checked. Remaining release/environment work is immutable producer
repinning, the PC native browser candidate rebuild and real PostgreSQL DDL/query-plan validation.
These are recorded as blocked evidence and are not silently upgraded by focused or Mock proof.
