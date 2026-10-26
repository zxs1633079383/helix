# Implementation Plan: Timeline V3 Delta Window

**Branch**: `046-timeline-v3-delta-window` | **Date**: 2026-07-28 | **Spec**:
[`spec.md`](spec.md)

**Input**: Helix-migrated feature specification plus the current Helix, cses-client and
mobile-qucik-c++ source locks recorded in [`discovery.md`](discovery.md).

## Summary

Replace Timeline full-window patches with a normalized, revisioned V3 Snapshot/Delta contract owned
by `helix-im`. Initial, older, newer and locate requests share a frontend-selected `pageSize`
(`20` default, `1..=60`); locate includes its target at index `floor(pageSize/2)` when both sides
are available. Helix producer Gates and canonical JSON are completed first, then PC and Mobile
consume the same byte-exact assets through operation-level tests. Full-window head eviction,
pixel anchor and new-message hint are bounded consumer state; they never become Helix delete/trim
operations or implicit window activation.

## Technical Context

**Language/Version**: Rust workspace toolchain; Angular 20/TypeScript in cses-client; C++17/QuickJS
in Mobile

**Primary Dependencies**: `helix-core` sans-IO ports, `helix-im` projector/module, host/FFI drivers,
Angular signals/MobX and WDIO, Mobile `SurfaceRuntime` and GoogleTest

**Storage**: Helix host SQLite is the local message/bounds authority; normalized Timeline Cache and
RenderWindow are expendable consumer state

**Testing**: Rust unit/integration producer tests and fixture generators; Jest/Angular focused tests
and WDIO mock/single-real/dual-real; GoogleTest/QuickJS; Strict Matrix/parity/hard-cut scripts

**Target Platform**: native PC/Tauri, Flutter FFI Mobile; shared contract remains Web-compatible
without claiming Web Timeline delivery

**Project Type**: cross-repository deterministic business kernel plus two platform consumers

**Performance Goals**: Frame apply `O(k + r)` for `k` operations and touched Range keys `r`;
single-key Upsert/append amortized `O(1)`; no scan across unrelated channels or entire entity cache

**Constraints**: no runtime dependency in `helix-core`; no Timeline Replace in V3; atomic
epoch/revision apply; no hand-edited consumer business fixture; page/window count bounded to 60;
preserve unrelated dirty work in all three repositories

**Scale/Scope**: 13 existing Timeline Gate families plus four new formal Gates; schema/producer,
canonical fixture, PC WDIO and Mobile GTest migration; Web consumer and long-duration performance
testing excluded

## Constitution Check

*GATE: Passed before Phase 0 and re-checked after Phase 1.*

| Principle | Design response | Verdict |
|---|---|---|
| Runtime-agnostic layering | Timeline identity/order/range/revision stay in `helix-im`; SQLite, DOM, scroll pixels and QuickJS stay at adapters/consumers | PASS |
| Deterministic effects and persistence | Bounds and visible persistence claims follow correlated storage replies; frontend supplies only bounded count | PASS |
| UI business truth | Timeline V3 is a closed typed operation set; platform consumers mechanically validate/apply it | PASS with governed V3 amendment |
| Evidence before completion | Producer, JSON/provenance, PC, Mobile, parity and hard-cut are separate validators | PASS |
| Minimal governed migration | Only Timeline changes schema; non-Timeline V2 surfaces remain legal; no dual-write fallback | PASS |
| Spec Kit loop admission | R1-R3 precede source changes; R4 uses recorded outputs, not static claims | PASS |
| Bounded hot paths | `pageSize <= 60`, per-Range mutation, indexed entities and chunk eviction prevent unbounded scans | PASS |

The Constitution still names InteractionFrame V2 as its historical UI authority. This Feature does
not weaken that authority: it performs the explicitly governed, Timeline-only V3 schema amendment
required by the source PRD while retaining V2 for unaffected surfaces. No implementation begins
until R3 admits that amendment.

**Approved convergence**: G-10b/G-11a production admission requires replacing Go's `createAt`-only
newer boundary with `(createAt, temporaryId)` composite cursor/order and a matching manual
PostgreSQL index script. The user approved this fourth-repository scope on 2026-07-28. Direct
projector fixtures remain L0 until Go, Helix production ScenarioRunner and provenance pass.

## Loop Detail Contract

### Inputs and Context

- **I-001**: `UiEvent`/command for latest, older, newer or locate carries optional `pageSize`;
  command actor/scope/action refs are already validated.
- **I-002**: Helix storage returns ordered committed messages and directional bounds for the
  requested channel. Local lookahead or an existing server remaining-count field proves exhaustion;
  neither becomes render data.
- **I-003**: Inbound WS/command updates contain a stable `temporaryId`; `createAt` and
  `temporaryId` form immutable `EntityKey`.
- **I-004**: Consumer apply starts from a matching Timeline epoch/base revision or from an empty
  cache when applying a Snapshot.
- **I-005**: Canonical fixtures originate from the locked Helix producer path and carry raw and
  semantic provenance; PC/Mobile inputs are byte-exact copies.

### Action Chain

- **A-001**: latest request -> validate/default `pageSize` -> query newest `N+1` -> persist/read
  bounds -> emit Snapshot with at most `N` entities and latest Range.
- **A-002**: older/newer request -> validate active Range/cursor -> query local lookahead or consume
  fixed 20-item server chunks for at most `ceil(N/20)` rounds -> emit Upserts, edge InsertKeys and
  SetRangeBounds -> consumer atomically expands the Range.
- **A-003**: uncached locate -> query target plus balanced context -> reassign short-side quota ->
  emit Upserts + InstallSegment + ActivateWindow(current token).
- **A-004**: ordinary send/update/revoke -> persist stable entity -> emit Upsert and, only for a new
  key, edge InsertKeys -> never Replace or implicitly Activate.
- **A-005**: live append while away from bottom -> apply entity/Range/watermark -> preserve active
  window and pixel anchor -> increment local hint -> explicit jump action selects latest and clears
  hint.
- **A-006**: full RenderWindow append -> append tail key -> locally evict visible head to `N` while
  retaining normalized Cache according to pin/budget policy -> no producer delete/trim/revision.
- **A-007**: revision/epoch/range error -> reject the whole Frame -> keep last correct state ->
  coalesce recovery -> apply authoritative Snapshot before accepting later Delta.
- **A-008**: Helix Gate Green -> export `.frame-v3.json` plus provenance -> byte-exact PC sync and
  operation/DOM assertions -> byte-exact Mobile sync and operation/window assertions -> parity and
  hard-cut.

### State Contract

- **S-001**: `Cold | Resident(snapshot)` -> ordered valid Delta -> `Resident(next_revision)`;
  mismatched Delta -> `RecoveryPending(last_good)`.
- **S-002**: `FollowLatest` -> user scrolls away/locates -> `Anchored(anchor, pending=0)`;
  append -> `Anchored(anchor, pending+1)`; explicit jump -> `FollowLatest(pending=0)`.
- **S-003**: `LatestRange | LocatedRange(token)` -> directional page -> same active intent with
  expanded Range; stale token response may update Cache but cannot change active window.
- **S-004**: `Pending(temporary EntityKey)` -> sent/failed/recalled update -> same EntityKey and one
  row; no second entity is representable.
- **S-005**: `pageSize missing | 1..=60 | outside` -> `20 | accepted | rejected before I/O`.
- **S-006**: `Gate producer pending -> Helix Green -> PC Green -> Mobile Green -> parity/hard-cut
  Green`; later phases cannot synthesize evidence for an earlier state.

### Validators

- **V-001**: Spec placeholder/count/checklist and mandatory loop R1 prove the external contract is
  complete before planning.
- **V-002**: focused Rust contract/projector/Gate tests prove schema validation, stable identity,
  page policy, operation sequence, stale token and recovery behavior.
- **V-003**: Helix fixture inventory/provenance validators prove every canonical file was generated,
  hashed and declared from the producer revision.
- **V-004**: cses-client codec/runtime/component/Jest and WDIO Gate contracts prove atomic V3 apply,
  bounded window, anchor/hint/jump and operation-level DOM behavior.
- **V-005**: Mobile `SurfaceRuntime` GoogleTests and Gate classes prove equivalent normalized state,
  page policy, anchor/hint/jump and V2 Timeline rejection.
- **V-006**: byte parity plus semantic parity prove Helix/PC/Mobile consume the same operations,
  keys, ranges, bounds, watermarks and revisions.
- **V-007**: Strict Matrix validator proves epoch 12 contains 50 cases, 47 active and three excluded,
  with G-10b/G-10c/G-10d/G-11a declared rather than hidden.
- **V-008**: three-repository hard-cut scans prove positive Timeline Replace producers/consumers are
  zero while unaffected non-Timeline V2 remains valid.
- **V-009**: `git diff --check`, focused build/test outputs and dirty-path inventory prove changes
  are reviewable and do not overwrite unrelated user edits.

## Project Structure

### Documentation (this feature)

```text
specs/046-timeline-v3-delta-window/
├── spec.md
├── discovery.md
├── plan.md
├── research.md
├── data-model.md
├── quickstart.md
├── verification.md
├── review-ledger.json
├── contracts/
│   ├── timeline-v3.md
│   └── gate-fixtures.md
├── evidence/
└── tasks.md
```

### Source Code (three repository roots)

```text
# helix
crates/helix-im/src/
├── interaction_frame.rs
├── projector/
├── repository/
└── handlers/
crates/helix-im/tests/
├── support/
├── fixtures/{g01,...,g10b-wdio,g10c-wdio,g10d-wdio,g11a-wdio}/
└── *_fixture_contract_test.rs
crates/helix-im/docs/Strict Matrix/

# cses-client
src/pages/message-v4/surface-runtime/
src/pages/message-v4/components/
tests/e2e/wdio/
├── fixtures/
├── harness/
├── pageobjects/
├── reporters/
└── specs/
scripts/message-wdio/

# mobile-qucik-c++
cpp/
tests/gtest/surface_runtime/
tests/fixtures/
scripts/
```

**Structure Decision**: Extend each repository's existing producer/runtime/Gate layout. Timeline
normalization is a focused module behind the existing SurfaceRuntime boundary; no fourth shared
runtime or cross-repository generated-code framework is introduced.

## Delivery Phases

1. **P0 Contract lock**: finish R1-R3, page policy, Gate IDs, Strict Matrix epoch decision and
   canonical file inventories.
2. **P1 Existing Helix Gates**: remove Replace/extra activation from 13 existing Timeline Gate
   families and regenerate truthful V3 provenance.
3. **P2 New Helix Gates**: implement G-10b/c/d/G-11a scenario/test classes, page policy, locate and
   selector semantics; generate all declared canonical assets.
4. **P3 PC Consumer**: byte-exact sync, V3 codec/cache/range/runtime, render-window selector,
   anchor/hint/jump, focused tests and four WDIO Gate families.
5. **P4 Mobile Consumer**: byte-exact sync, equivalent runtime state machine and four GTest classes.
6. **P5 Cutover**: epoch-12 Strict Matrix, parity/hard-cut, focused real lanes where prerequisites
   exist, R4 evidence and physical Timeline V2 cleanup only after all positive gates pass.

## Complexity Tracking

No constitutional layering exception is required. The cross-repository scope is intrinsic to the
already-decided producer-owned canonical contract; duplicating Timeline business logic in each
consumer would be the rejected, more complex alternative.
