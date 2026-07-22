<!--
Sync Impact Report
- Version change: template -> 1.0.0
- Ratified principles: runtime-agnostic layering, deterministic effect flow,
  InteractionFrame V2 authority, evidence-first validation, and minimal governed change.
- Templates checked: the generic plan/spec/tasks templates already require a
  constitution gate and task traceability; no template text needs a Helix-specific rewrite.
- Follow-up: every active feature plan must replace any template-constitution
  fallback with these executable gates before implementation starts.
-->

# Helix Constitution

## Core Principles

### I. Runtime-Agnostic Layering

`helix-core` is a sans-IO deterministic host and never imports platform runtime,
database, HTTP, WebSocket, renderer, or IM schema dependencies. Business semantics
belong to a module such as `helix-im`; drivers and PC/mobile/Web shells implement
ports and rendering at the edge. Dependencies always point adapter -> core, never
the reverse.

### II. Deterministic Facts, Effects, And Persistence

Business state changes only from explicit Action, HostObservation, inbound, timer,
or PortReply ticks. Time and identifiers are host supplied. A module requests I/O
through Effects and treats a correlated port reply as a fact. Visibility that claims
durable business state appears only after the corresponding `PersistOk`; HTTP
transport acceptance is never canonical business confirmation by itself.

### III. InteractionFrame V2 Is The Only UI Business Truth

`helix-im` owns typed surface semantics, ActionBinding, SurfaceComposer, and
InteractionFrame V2. A platform shell may retain local input, focus, animation, and
an expendable frame cache, but it must not recreate message reconciliation, ordering,
permissions, unread, or raw-event business reducers. Frames are validated as a
whole and committed atomically before `FrameApplied`; directives run after that
commit and acknowledge independently. Unknown schema, surface, epoch, revision, or
content fails closed and never changes a cache slot.

### IV. Evidence Before Completion

Every behavior claim has the evidence plane that can falsify it: deterministic Rust
tests for tick/effect/persist order, PC or mobile renderer/DOM tests for frame
consumption, and only when requested a bounded read-only SQLite/PG or real-chain
run. Correlation keys cross action ingress, tick, effect, persistence, composition,
frame, cache, and acknowledgement. A missing evidence plane is `blocked` or
`inconclusive`, never a fabricated pass; reports record actual command start, end,
duration, retry, skip, and blocker.

### V. Minimal, Governed Migration

Change only the smallest coherent contract and implementation surface. New content
kinds extend a tagged, versioned registry rather than widening unrelated root VMs.
No direct SQL repair, producer dual-write, raw reducer fallback, or speculative
framework is allowed. A legacy raw producer, fixture, subscription, or documentation
reference is physically removed exactly once, only after the applicable V2/27-Gate
review accepts the recorded deletion inventory; final regression evidence follows
the deletion.

## Constraints And Safety

Hot paths use bounded work: atomic frame application is `O(k)` in mutations and
window replacement stays within its declared window bound. Do not add unbounded raw
JSON copies, full replay, cross-surface business joins, or read-modify-write loops.
Cross FFI/Web boundaries carry batches, not one callback per effect. External input
does not use `unwrap` or `expect`; FFI never unwinds across an ABI boundary.

ActionBinding is opaque, actor- and scope-bound, epoch/revision validated, and
request scoped. A component cannot choose channel, user, or temporary message IDs.
Read-only database tools are evidence tools only; a state repair must use the normal
application Effect path. Sensitive fields stay out of logs unless a documented,
bounded diagnostic requires them.

## Development Workflow And Quality Gates

Before implementation, the feature has a spec, plan, data model, wire contract,
task list, and Constitution Check with no unresolved template placeholders. Tasks
are executed in dependency order, tests precede their implementation, and completed
tasks are marked in the task list only after their stated evidence exists. Work that
exceeds 30 minutes reports completed work, current evidence, blocker, and next step.

Worktrees isolate each repository touched by a feature. Preserve pre-existing dirty
changes, record branch identity before and after worktree operations, and do not
claim cross-repository completion from a single checkout. Code changes obey the
repository harnesses and focused validation first; wider or real-chain gates run only
when their required environment and pins are available.

## Governance

This Constitution is the Spec Kit interpretation of Helix `AGENTS.md`,
`docs/GOAL.md`, `docs/ROADMAP.md`, and the active InteractionFrame contracts. When
those higher-level project rules change, update this document and affected feature
plans in the same review. A constitutional amendment requires a version change,
written rationale, migration impact, and explicit human approval before it governs
new work. Each implementation review verifies these principles, records any justified
exception, and rejects a plan that silently weakens them.

**Version**: 1.0.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-07-21
