# Research: Timeline V3 Delta Window

## R-001: Window size policy

**Decision**: Use one optional `pageSize` on latest/older/newer/locate commands. Default to 20,
accept `1..=60`, and reject outside values before storage or transport work.

**Rationale**: The user explicitly requires frontend control and identical adaptive behavior. A
single value prevents page, resident and visible-window counts from drifting.

**Alternatives considered**: Fixed 20 rejects the requested 60 mode; separate per-direction sizes
make locate and bounded eviction ambiguous; unbounded client sizes violate hot-path constraints.

## R-002: Locate balancing

**Decision**: The target counts toward `N`. Request `floor(N/2)` older and
`N-1-floor(N/2)` newer records. If one side is short, refill from the other side until `N` or both
bounds are exhausted. Activate only the current navigation token.

**Rationale**: It gives a deterministic center for even and odd sizes and wastes no available quota.

**Alternatives considered**: Excluding the target returns `N+1`; equal halves are impossible for
even totals; no refill produces undersized windows when data is available.

## R-003: Full-window append

**Decision**: Helix emits only Upsert/Insert and necessary bound/watermark changes. The active
consumer RenderWindow evicts its visible head to remain at `pageSize`; normalized Cache eviction is
separately governed by chunk budget and pins.

**Rationale**: Visible-window eviction is presentation/cache policy, not a business deletion.

**Alternatives considered**: Adding a `trim` Delta operation widens the protocol and makes local
eviction advance business revision; full Snapshot/Replace destroys stable row identity.

## R-004: Away-from-bottom state

**Decision**: Represent consumer selection as `FollowLatest` or `Anchored(anchor, pendingCount)`.
Incoming latest deltas never activate the latest window while Anchored. An explicit jump action
selects latest and clears the hint.

**Rationale**: Helix continues to own ordered facts while pixel offset and hint interaction stay at
the platform boundary. The same state transition can be tested on PC and Mobile.

**Alternatives considered**: Producer auto-activation causes scroll jumps; encoding pixels in the
Frame breaks cross-platform semantics; deriving hint only from DOM is not portable.

## R-005: Formal Gate denominator

**Decision**: Add G-10b, G-10c, G-10d and G-11a as distinct Strict Matrix cases and bump epoch 11 to
12. Expected denominator becomes 50 total, 47 active and three existing excluded.

**Rationale**: The new behaviors add independent failure surfaces and user explicitly requested
formal Gates. Hiding them under G-10/G-11 would preserve an obsolete denominator.

**Alternatives considered**: Reusing G-10/G-11 names hides missing evidence; adding docs without the
machine matrix lets reports omit the cases.

## R-006: Canonical fixture ownership

**Decision**: Helix production scenario paths generate `.frame-v3.json`. Provenance inventories raw
SHA-256 and a semantic SHA-256 covering operation order, EntityKey, Range location, Bounds,
watermark, revision and active window. PC/Mobile only byte-copy declared files.

**Rationale**: Producer ownership prevents consumer tests from inventing a contract they already
know how to pass.

**Alternatives considered**: Hand-authored fixtures cannot prove producer behavior; a final-view
hash misses Replace or activation regressions.

## R-007: Existing Gate migration order

**Decision**: First migrate G-01/G-02/G-03a/G-04/G-04a/G-05/G-06/G-07/G-10/G-10a/G-11/G-14/G-14a,
then add the four formal Helix Gates, then PC, then Mobile.

**Rationale**: The current dirty tree already contains partial schema-3 assets with stale names,
provenance and extra activation. New consumer work before producer convergence would freeze drift.

**Alternatives considered**: Runtime-first or consumer-first migration permits green tests over
obsolete V2 behavior.

## R-008: Hard cut

**Decision**: Timeline V3 Consumers reject Timeline in schema-v2 Frames. Once positive producer and
consumer gates pass, remove Timeline `timelinePatch + replaceView` code/fixtures physically while
retaining V2 for unaffected surfaces.

**Rationale**: Runtime negotiation and dual paths multiply ambiguous behavior. Timeline-only
hard-cut honors the PRD without forcing unrelated surface migration.

**Alternatives considered**: Long-lived dual-read/fallback masks incomplete migration; global V2
removal exceeds scope.

## R-009: Evidence boundaries

**Decision**: Track producer contracts, canonical provenance, PC runtime/DOM, Mobile runtime,
byte/semantic parity, persistence and real lanes independently. Only the evidence plane a validator
actually observes can pass.

**Rationale**: Static/fixture/mock success does not prove persistence, WS, native driver or rendered
behavior.

**Alternatives considered**: One aggregate boolean encourages evidence substitution and conflicts
with Strict Matrix fail-closed reporting.
