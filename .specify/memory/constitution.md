<!--
Sync Impact Report
- Version change: 1.1.0 -> 2.0.0
- Ratified principles: runtime-agnostic layering, deterministic effect flow,
  MessageV3 Gate Event authority, evidence-first validation, per-Gate hard-cut migration,
  and Spec Kit loop-state admission.
- Architecture amendment: InteractionFrame V2, Surface, ActionBinding, UiDirective,
  FrameApplied, and legacy V1 inbound/outbound are no longer target contracts.
- Added project extension: helix-loop supplies the persistent ledger, invalidation
  graph, validators, and negative tests that enforce these gates.
- Migration owner: specs/047-im-event-gate-inventory; existing feature bundles remain
  historical evidence and must not reintroduce the retired contracts.
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

### III. MessageV3 Gate Events Are The Only Cross-Platform Business Truth

`helix-im` owns each Gate's deterministic `Effect::Emit {event,data}` contract.
An emitted `data` value is computed from authenticated inbound authority and local
facts read after the matching `PersistOk`; it is not a transport acknowledgement,
database row guessed from wire JSON, or platform projection. Native, FFI, and Web
drivers deliver the same event bytes in a batch or channel without wrapping them in
InteractionFrame, Surface, ActionBinding, UiDirective, FrameApplied, or a platform
business VM. A shell may retain local input, focus, animation, and expendable display
cache, but it must not recompute reconciliation, ordering, permissions, unread, or
viewer state from multiple events.

The ignored `.v1` directory under Spec 047 is an evidence/schema version only and
does not name a runtime V1 protocol. Canonical Gate JSON comes from production
`ImModule::handle` replay tests; PC WDIO Mock and Mobile GTest consume byte-exact
mirrors rather than hand-maintained payloads.

### IV. Evidence Before Completion

Every behavior claim has the evidence plane that can falsify it: deterministic Rust
tests for tick/effect/persist order and emitted bytes, PC WDIO or Mobile GTest for
canonical event consumption, and only when requested a bounded read-only SQLite/PG
or real-chain run. Correlation keys cross command ingress, tick, effect, persistence,
post-PersistOk readback, emit, driver delivery, and application observation.
A missing evidence plane is `blocked` or
`inconclusive`, never a fabricated pass; reports record actual command start, end,
duration, retry, skip, and blocker.

### V. Minimal, Governed Migration

Change only the smallest coherent Gate contract and implementation surface. Migration
is a vertical hard cut per Gate: first connect the Helix production replay test,
real `ImModule::handle` path, and canonical JSON generation; then physically remove
that Gate's Helix V2 Frame and legacy V1 producer, parser, subscription, fixture,
capability entry, reducer, fallback, and documentation reference. A migrated Gate
never dual-writes or dual-consumes old and new contracts. The next Gate starts only
after the current Gate's deletion inventory and focused Helix evidence pass. A
feature may explicitly defer PC WDIO/Mobile GTest fixture mirroring to a later
version, but generated provenance and hashes must make that later sync byte-exact.
Final whole-repository zero-reference gates follow the last migrated Gate.

### VI. Spec Kit Loop Admission

Spec Kit remains the author of `spec.md`, `plan.md`, and `tasks.md`. Each feature
also has a source-discovery record, a persistent review ledger, and durable
verification evidence. The ledger is the sole workflow state: it records input
fingerprints, gate verdicts, the next allowed action, and bounded retry history.
The loop may advance only through a passing independent validator.

Reviews have one owner and one purpose: R1 validates behavior and completion,
R2 validates design, R3 validates task traceability, and R4 validates implemented
evidence. An unchanged fingerprint reuses its prior verdict. A changed source,
specification, plan, task list, or verification artifact invalidates only the
dependent downstream gates. The engine returns to the earliest invalid phase; it
does not rerun a whole feature review by habit.

Every feature records trigger, goal, completion, inputs and context, action,
validator, workflow state, boundaries, and any human gate exactly once in the
artifact that owns it. A business state machine belongs in the plan; workflow
state belongs only in the review ledger. A repeated failure with the same
task/validator signature may receive at most two automatic repair attempts.
The next recurrence enters `NeedsHuman`.

## Constraints And Safety

Hot paths use bounded work: one inbound authority produces a bounded Effect batch,
and window replacement stays within its declared window bound. Do not add unbounded
JSON copies, full replay, cross-event platform joins, or read-modify-write loops.
Cross FFI/Web boundaries carry batches, not one callback per effect. External input
does not use `unwrap` or `expect`; FFI never unwinds across an ABI boundary.

Commands are actor-, tenant-, and request-bound; a component cannot override
authenticated viewer identity or derive channel/user/message authority outside the
Gate contract. Read-only database tools are evidence tools only; a state repair must
use the normal application Effect path. Sensitive fields stay out of logs unless a
documented, bounded diagnostic requires them.

## Development Workflow And Quality Gates

Before implementation, the feature has a spec, plan, data model, wire contract,
per-Gate deletion inventory, task list, source discovery, valid R1-R3 ledger gates,
and Constitution Check with no unresolved template placeholders. Tasks are executed
in dependency order, tests precede their implementation, and completed tasks are
marked in the task list only after their stated evidence exists. Work that
exceeds 30 minutes reports completed work, current evidence, blocker, and next step.

Worktrees isolate each repository touched by a feature. Preserve pre-existing dirty
changes, record branch identity before and after worktree operations, and do not
claim cross-repository completion from a single checkout. Code changes obey the
repository harnesses and focused validation first; wider or real-chain gates run only
when their required environment and pins are available.

## Governance

This Constitution is the Spec Kit interpretation of Helix `AGENTS.md`,
`docs/GOAL.md`, `docs/ROADMAP.md`, and the active MessageV3 Gate Event contracts. When
those higher-level project rules change, update this document and affected feature
plans in the same review. A constitutional amendment requires a version change,
written rationale, migration impact, and explicit human approval before it governs
new work. Each implementation review verifies these principles, records any justified
exception, and rejects a plan that silently weakens them.

**Version**: 2.0.0 | **Ratified**: 2026-07-21 | **Last Amended**: 2026-07-29
