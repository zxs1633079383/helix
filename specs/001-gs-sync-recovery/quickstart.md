# Quickstart: 验证 GS01-GS04 同步恢复

## Goal

Run one isolated tenant/channel recovery from a known behind cursor to remote authority head, prove atomic local commit and V2 UI recovery, then run the failure and identity/unknown-channel counterexamples. The run is successful only when every required evidence plane passes.

## Preconditions

1. Work in an isolated test tenant/channel and use pseudonymous IDs in exported evidence.
2. Confirm the current target source and exact tests before running; this feature has no right to mutate remote PG or local SQLite manually.
3. Confirm readonly MCP capability and discover the real local schema. The planned `channel_event_ledger` and `channel_sync_coverage` must exist before attempting the event comparison.
4. Capture a correlation ID at connect/login and preserve it in Helix, Go and PC log filters.
5. When work passes 30 minutes, emit the progress record defined in [the evidence contract](./contracts/sync-recovery-evidence.md).

## Phase A: Static and deterministic evidence

Run the focused tests after implementation, recording start/end/wall clock:

```bash
cargo test -p helix-im --test gs01_04_v2_sync_recovery_test
cargo test -p helix-driver-host --test im_event_persist
```

Expected results:

- reconnect below head produces `Compare -> Syncing -> PersistAtomic -> PersistOk -> Frame v2`;
- equal sequences do not add facts/ledger coverage or manufacture a Frame;
- injected storage failure changes neither cursor nor final Frame;
- duplicate/out-of-order events are idempotent or request authority recovery;
- identity A -> B invalidates A's scope/action refs;
- unknown channel has no enabled send action before hydration and receives summary+composer atomically after it.

Run the focused PC DOM suite from the cses-client repository:

```bash
cd /System/Volumes/Data/workspace/angular/cses-client
npx jest src/pages/message-v4/surface-runtime/__tests__/gs01-04-sync-recovery-dom.test.ts --runInBand
```

Expected results: global syncing appears while recovery is pending, dialog/unread become visible from one applied Frame, a hydrated channel gains its composer binding only after commit, and an invalid Frame causes resync rather than raw-event rendering.

## Phase B: Readonly data evidence

1. Use local SQLite MCP to list tables and verify the fact tables, `channel_event_cursor`, `channel_event_ledger`, and `channel_sync_coverage`.
2. Read the selected local channel's cursor, coverage rows, ledger range, message rows and members with strict `tenant_actor_scope`, `channel_id`, `event_seq` predicates and a limit. The recovery ledger scope is an opaque, length-prefixed tenant/actor key; do not split or reconstruct it in a query client.
3. Use PG MCP to read the corresponding `channel_event_head`, `channel_event`, `post`, `channel`, and `channel_member` rows over the same range.
4. Compare identity, event range/count/hash, final cursor and documented fact mappings. Save query shape, not credentials or unbounded data.

Pass requires all of the following:

```text
local cursor == remote authority head
local coverage [from_seq,to_seq] == selected remote event range
ledger PKs/hash count == selected remote events exactly once
local projected messages/channel/members == remote facts under documented mapping
```

## Phase C: Runtime and UI chain

1. Start from a local committed cursor lower than the authority head through an approved fixture/setup, not direct DB modification.
2. Establish/re-establish WS and collect correlation-filtered Helix, Go and PC logs.
3. Observe global syncing, persist receipt, composed V2 Frame, atomic cache apply and ack.
4. Repeat with equal sequences, persistence failure, identity switch and unknown channel hydration.
5. Use the shared V2 Frame fixture on mobile. It must produce the same slot-level apply/ack or scoped resync result as PC; no mobile raw event reducer is allowed.

## Evidence Pack Checklist

- deterministic test outputs with command times;
- PC DOM output and mobile fixture result;
- readonly local/remote query extracts and mapping summary;
- correlation hop timeline;
- current source pin and configured test tenant;
- `PASS`, `FAIL`, or `BLOCKED` with the first deviation;
- 30-minute progress entries for long runs.

## Stop And Escalate

Stop and request human intervention for local-ahead-of-remote, unresolvable authority conflict, missing readonly access, any need for DB write/DDL in a real environment, real-user data exposure, or a request to change the signed-off V2 interaction semantics. Do not “repair” evidence by editing SQLite, PG or the Frame cache.

## Readonly Query Parameters

Use `:tenant_actor_scope` only for `channel_event_ledger` and `channel_sync_coverage`; it is the
exact opaque result of the active candidate's `recovery_tenant_actor_scope(tenant, actor)`.
`channel_member.team_id` remains `:tenant`, because it is a source fact rather than recovery
audit partitioning. Keep every query bounded by `:channel`, `:from_seq`, `:to_seq`, and
`1 <= :limit <= 500`; read-only MCP is validation evidence, never a migration or repair tool.
