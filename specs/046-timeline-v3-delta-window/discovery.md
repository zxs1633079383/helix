# Source Discovery

**Feature**: `specs/046-timeline-v3-delta-window`
**Source Lock**: `86b6a03fb66692eadcbe78ab7d8c179fa289144a`
**Input Lock**: cses-client `3000e97feb72039e4f9d30aa20a982f6e86c18c1`
**Worktree State**: Both repositories are dirty. Timeline V3 Rust and client changes are
uncommitted work in progress and are evidence of current shape only, not an accepted baseline.

## Facts *(mandatory)*

- **F-001**: The source Feature Spec defines eight prioritized user stories, 47 functional requirements and 17 measurable outcomes covering latest-window mount, bounded live updates, bidirectional paging, locate, bounded cache, recovery, V2 hard-cut and cross-client fixture migration. | **Evidence**: `/System/Volumes/Data/workspace/angular/cses-client/specs/20260727-214840-timeline-v3-delta/spec.md:1`
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

## Unknowns

- **U-001**: Which Strict Matrix Gates and fixture families form the exact delivery denominator for
  this cross-cutting Timeline migration? | **Resolution**: Resolve from the active Matrix and
  producer/consumer inventories during `/speckit-plan`; do not derive completion from the story list.
- **U-002**: Which current dirty Timeline V3 changes are intentional implementation versus
  incomplete experiments, and which have passing focused tests? | **Resolution**: Lock diffs and
  verification outputs during planning before any implementation task is admitted.
- **U-003**: How the product-level latest page size of 20 composes with Helix's current bounded
  60-item attached window is not established by the source inputs. | **Resolution**: Specify page,
  resident window and render-window bounds independently in `plan.md`, then add falsifiable tests for
  each bound.
- **U-004**: PC and Mobile byte-exact fixture parity, normalized state parity and real rendering
  evidence have not been rerun against one fixed Helix revision. | **Resolution**: Treat each evidence
  plane as pending until a plan-defined command produces bounded artifacts.
- **U-005**: Product-level Web Timeline consumption is absent from the source PRD. | **Resolution**:
  This Feature keeps the shared contract Web-compatible but explicitly excludes Web Consumer
  delivery and therefore cannot claim `ThreePlatformGreen`; a later Feature must own that closure.

## Decisions

- **D-001**: Preserve the source Spec's eight stories, 47 requirements and 17 outcomes as the
  behavioral baseline, while adopting the current Helix Spec Kit template and Loop Contract.
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
