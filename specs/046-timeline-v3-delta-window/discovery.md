# Source Discovery

**Feature**: `specs/046-timeline-v3-delta-window`
**Source Lock**: `86b6a03fb66692eadcbe78ab7d8c179fa289144a`
**Input Lock**: cses-client `3000e97feb72039e4f9d30aa20a982f6e86c18c1`
**Worktree State**: Both repositories are dirty. Timeline V3 Rust and client changes are
uncommitted work in progress and are evidence of current shape only, not an accepted baseline.

## Facts *(mandatory)*

- **F-001**: The source Feature Spec defines eight prioritized user stories, 47 functional requirements and 17 measurable outcomes covering latest-window mount, bounded live updates, bidirectional paging, locate, bounded cache, recovery, V2 hard-cut and cross-client fixture migration; the Helix copy extends those inputs with the user-approved adaptive window and four formal Gate contracts. | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/specs/20260727-214840-timeline-v3-delta/spec.md:1`
- **F-002**: The source PRD is marked decided/pending implementation, assigns Timeline contract
  authority to Helix, and scopes delivery to Helix, PC and Mobile. | **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/docs/2026-07-27-message-v4-timeline-v3-prd.md:3`
- **F-003**: The source PRD requires initial/latest loading in pages of 20, stable
  `(createAt, temporaryId)` identity, bounded Delta for ordinary changes, anchored reading away
  from the bottom, persistence-owned bounds, atomic revision recovery and eventual Timeline V2
  removal. | **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/docs/2026-07-27-message-v4-timeline-v3-prd.md:39`
- **F-004**: The source PRD explicitly leaves all non-Timeline surfaces on their current contracts,
  forbids client-side business reordering and runtime V2 fallback, and excludes long-running or
  million-message load testing from this feature. | **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/docs/2026-07-27-message-v4-timeline-v3-prd.md:52`
- **F-005**: Helix's current north star makes `helix-im` the owner of cross-platform ordering,
  reconciliation, windows and interaction semantics, while platform shells only render and execute
  platform concerns. | **Evidence**: `docs/GOAL.md:8`
- **F-006**: Helix's publishable three-platform metric requires PC, Mobile and Web evidence from the
  same revision and evidence window; PC/Mobile completion alone cannot be described as
  `ThreePlatformGreen`. | **Evidence**: `docs/GOAL.md:72`
- **F-007**: The dirty Helix working tree already models a stable Timeline entity key as
  `createAt + temporaryId`, normalized entities, ordered ranges and an active window that references
  ranges rather than copying a second message array. | **Evidence**:
  `crates/helix-im/src/interaction_frame.rs:1199`
- **F-008**: The dirty Helix working tree exposes the six closed Timeline Delta operations required
  by the input contract: upsert, install segment, insert keys, merge ranges, set bounds and activate
  window. | **Evidence**: `crates/helix-im/src/interaction_frame.rs:1282`
- **F-009**: The dirty Helix working tree rejects Timeline in schema-v2 Frames and requires
  per-surface schema versions in schema-v3 Frames, preserving legacy surface schema for non-Timeline
  mutations. | **Evidence**: `crates/helix-im/src/interaction_frame.rs:611`
- **F-010**: The dirty Helix projector has a bounded in-memory Timeline window and emits a complete
  Snapshot for attach plus Timeline Delta for attached updates; its current constant is 60 items,
  so the source requirement for a latest page of 20 still needs design and verification rather than
  being inferred as complete. | **Evidence**: `crates/helix-im/src/projector/timeline.rs:22`
- **F-011**: The dirty Frame composer selects schema v3 whenever a Timeline mutation is present and
  schema v2 otherwise, then assigns Timeline surface schema v3 and legacy schema v2 to other
  surfaces in a mixed transaction. | **Evidence**: `crates/helix-im/src/projector/mod.rs:172`
- **F-012**: The feature's source inputs and both implementation repositories are currently dirty;
  fixture, consumer, real-chain and three-platform completion cannot be established from source
  inspection alone. | **Evidence**: `git status --short` and
  `git -C /System/Volumes/Data/workspace/angular/cses-client status --short`
- **F-013**: Strict Matrix epoch 11 currently contains 46 cases, 43 active and three excluded; the
  requested G-10b/G-10c/G-10d/G-11a cases are absent, so adding them is a denominator and epoch
  change rather than a hidden extension of G-10/G-11. | **Evidence**:
  `crates/helix-im/docs/Strict Matrix/strict-matrix.v1.json`
- **F-014**: The current cses-client consumer still fixes Timeline at schema v2, accepts
  `timelineSnapshot` or `timelinePatch + replaceView`, and replaces the full view in its runtime;
  its load-newer, anchored hint and V3 Range cache behavior are not implemented. | **Evidence**:
  `/System/Volumes/Data/workspace/angular/cses-client/src/pages/message-v4/surface-runtime/surface-contract.types.ts:7`
- **F-015**: The current Mobile consumer still fixes Timeline at schema v2 and implements
  `timelineSnapshot`/`timelinePatch` plus `replaceView`; V3 entity/range/window state and the four
  formal GTest classes are absent. | **Evidence**:
  `/System/Volumes/Data/workspace/c/mobile-qucik-c++/third_party/helix/app/SurfaceRuntime.hpp:190`
- **F-016**: Existing Timeline fixture producers cover 13 affected business Gate families, while
  send/revoke and other ordinary updates still include legacy full-replace expectations or
  non-navigation activation in at least one producer/consumer asset. | **Evidence**:
  `crates/helix-im/tests/fixtures/README.md`
- **F-017**: Go's existing `/posts/getPostsAfterIndex` accepts only one `postIds` anchor, returns at
  most 20 newer posts plus an authoritative remaining count, and can therefore serve `pageSize=60`
  only through bounded stable-anchor rounds. | **Evidence**:
  `/System/Volumes/Data/workspace/golang/cses-im-server/internal/api/post/handlers_read.go:186`
- **F-018**: Helix's current production locate path stops when the target is absent from the attached
  Timeline and has no navigation generation, persistence barrier or target-context readback; direct
  `TimelineProjector` fixture generation does not prove that missing production chain. | **Evidence**:
  `crates/helix-im/src/port_reply/dispatch.rs:990`

## Unknowns

- **U-002**: Which current dirty Timeline V3 changes are intentional implementation versus
  incomplete experiments, and which have passing focused tests? | **Resolution**: Lock diffs and
  verification outputs during planning before any implementation task is admitted.
- **U-004**: PC and Mobile byte-exact fixture parity, normalized state parity and real rendering
  evidence have not been rerun against one fixed Helix revision. | **Resolution**: Treat each evidence
  plane as pending until a plan-defined command produces bounded artifacts.
- **U-005**: Product-level Web Timeline consumption is absent from the source PRD. | **Resolution**:
  This Feature keeps the shared contract Web-compatible but explicitly excludes Web Consumer
  delivery and therefore cannot claim `ThreePlatformGreen`; a later Feature must own that closure.
- **U-006**: The current Go newer-page store orders and filters only by `createAt`, so bounded
  20-item rounds can skip remaining messages with the same timestamp and cannot satisfy stable
  `(createAt, temporaryId)` pagination. | **Resolution**: APPROVED on 2026-07-28: include
  cses-im-server, add a stable `(createAt, temporaryId)` composite cursor/order plus an explicit
  PostgreSQL index/DDL script, then promote projector-only Gate JSON only after the production
  producer chain passes.

## Decisions

- **D-001**: Preserve the source Spec's eight stories as the behavioral baseline, then extend its
  47 requirements and 17 outcomes with the user-approved adaptive window and formal Gate delivery
  contracts while adopting the current Helix Spec Kit template and Loop Contract.
- **D-002**: Keep PC and Mobile as the consumer delivery scope because that is the decided source
  scope; record Web as a bounded follow-up rather than silently widening this Feature.
- **D-003**: Treat protocol operation names, entity identity, Frame/surface versions and canonical
  fixture semantics as externally observable contracts; keep source paths, data structures and
  implementation mechanisms out of `spec.md`.
- **D-004**: Do not promote dirty Rust/client changes, fixture presence, static scans or final-view
  similarity to completion evidence. R4 remains pending until real validators record the required
  evidence planes.
- **D-005**: Changes to entity identity, operation set, runtime fallback or physical V2 deletion are
  explicit human gates because they alter irreversible or cross-repository contracts.
- **D-006**: Name the four formal cases G-10b `LoadNewerGate`, G-10c
  `FullWindowAppendEvictionGate`, G-10d `AnchoredNewMessageGate` and G-11a
  `UncachedLocateBidirectionalGate`; bump the Strict Matrix epoch and denominator by four rather
  than folding them into existing cases.
- **D-007**: Use one frontend-controlled `pageSize` for initial, older, newer, locate and the active
  RenderWindow. It defaults to 20, accepts `1..=60`, rejects 0/61, and does not allow the frontend to
  define order, cursor, Range or identity.
- **D-008**: A located target counts toward `pageSize`; with data on both sides its index is
  `floor(pageSize/2)`, and unused quota on a short side is deterministically reassigned to the other
  side without exceeding the requested total.
- **D-009**: Full-window head eviction is local Cache/RenderWindow policy, not a Helix Timeline
  delete/trim operation. Away-from-bottom append must not activate latest; only an explicit consumer
  jump action may change the active latest window.
- **D-010**: Reject reuse of the current `/posts/getPostsAfterIndex` as the final stable paging
  contract: its `createAt > anchor.createAt` filter and single-column order can lose same-timestamp
  messages across 20-item rounds. The eventual Go contract must expose an equivalent stable
  composite cursor/order before G-10b or G-11a can become canonical.
- **D-011**: Locate/newer commands carry optional `pageSize` and a navigation token; when an older
  caller omits the token Helix deterministically derives it from `req_id`. Only the current
  scope-bound token can activate, while stale valid data may still persist and enter Cache.
- **D-012**: The user explicitly authorized cses-im-server as the fourth implementation repository
  and fixed server ordering to `(createAt ASC, temporaryId ASC)`. The migration supplies manual
  forward/rollback SQL for a matching PostgreSQL composite index; no production DDL is executed by
  this task.
