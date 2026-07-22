# Contract: GS01-GS04 Recovery Evidence

## Read-only Boundary

All state inspection is performed with `mcp__helix_local_sqlite_ro__*` and `mcp__cses_pre_postgres_ro__*`. Queries must be bounded by tenant, channel, sequence range and row limit. Do not use DB writes, DDL, cursor changes, ledger inserts, cache changes or application restarts to force an expected result.

## Evidence Record

Each candidate run writes one immutable evidence record outside business data:

| Field | Required evidence |
|---|---|
| identity | test tenant/actor pseudonym, session epoch, no secret/token. |
| scope | channel id(s), `local_before`, `remote_head`, expected `from_seq/to_seq`. |
| hops | connection/login, compare, HTTP/WS input, PersistAtomic receipt, compose, Frame apply, ack using one correlation. |
| local facts | bounded rows from `channel`, `channel_member`, `message`, `channel_event_ledger`, `channel_sync_coverage`, `channel_event_cursor`. |
| remote facts | bounded rows from PG `channel`, `channel_member`, `post`, `channel_event`, `channel_event_head`. |
| UI | PC DOM assertions for global syncing and recovered Surface VMs; mobile Frame fixture apply result. |
| tests | command, start/end timestamps, wall time, pass/fail/skip and first failure. |
| verdict | `PASS | FAIL | BLOCKED`, first divergence and next action. |

## Required Comparisons

| Comparison | Pass condition |
|---|---|
| authority head | local committed cursor equals remote head after successful recovery. |
| event range | ledger has exactly one canonical hash per remote `(channel_id,event_seq)` over selected coverage; count and range equal coverage. |
| facts | selected message/channel/member rows match remote canonical values or documented projection mapping. |
| atomic failure | injected persistence error leaves facts, ledger, coverage, cursor and final Frame at pre-batch state. |
| UI state | syncing is global during work, recovery state appears only after commit; unknown channel send action is disabled/absent until hydration commit. |
| isolation | actor B cannot see actor A's Frame slots, ledger scope or ActionBindings after relogin. |

## Failure Classification

| Condition | Verdict | Automatic response |
|---|---|---|
| local behind remote and recovery not complete | `FAIL` | retain evidence, retry authority recovery if contract permits. |
| local ahead of remote / invalid contiguous range | `BLOCKED` | stop automatic reconciliation; request human decision. |
| DB read permission/query limit missing | `BLOCKED` | report capability gap, do not downgrade to “passed by tests”. |
| Frame schema/revision/epoch invalid | `FAIL` | request scoped snapshot; do not consume raw emit. |
| persistence failure | expected negative test pass only if no cursor/Frame advance | record first failing write/hop. |
| requires DB write, touches real user data or changes signed-off semantics | `BLOCKED` | human intervention required. |

## Progress Contract

During continuous implementation or verification, at least every 30 minutes write a concise status line:

```text
<timestamp> | done: <completed evidence/work> | current: <result or blocker> | next: <one concrete operation>
```

This is not a timer, an auto-completion signal, or an acceptance substitute.

## Bounded Query Shapes (T002)

All placeholders are supplied by the isolated evidence run; never substitute an unbounded wildcard. `:tenant`, `:actor`, `:channel`, `:from_seq`, `:to_seq` and `:limit` are audit inputs, with `1 <= :limit <= 500`.

```sql
-- Local SQLite: committed cursor, coverage, ledger, facts. Run separately with named parameters bound by the readonly MCP caller.
SELECT channel_id, last_event_seq FROM channel_event_cursor
 WHERE channel_id = :channel LIMIT 1;
SELECT coverage_id, tenant_id, channel_id, from_seq, to_seq, event_count, facts_hash, correlation_id
  FROM channel_sync_coverage
 WHERE tenant_id = :tenant AND channel_id = :channel
   AND to_seq >= :from_seq AND from_seq <= :to_seq
 ORDER BY from_seq LIMIT :limit;
SELECT tenant_id, channel_id, event_seq, event_kind, message_id, event_hash, coverage_id
  FROM channel_event_ledger
 WHERE tenant_id = :tenant AND channel_id = :channel
   AND event_seq BETWEEN :from_seq AND :to_seq
 ORDER BY event_seq LIMIT :limit;
SELECT id, channel_id, user_id, type, create_at, update_at FROM message
 WHERE channel_id = :channel ORDER BY create_at, id LIMIT :limit;
SELECT channel_id, user_id, team_id, role, updated_at FROM channel_member
 WHERE channel_id = :channel AND team_id = :tenant ORDER BY user_id LIMIT :limit;
```

```sql
-- Remote PostgreSQL authority. The exact tenant predicate is selected from the verified table schema;
-- no query may omit channel and sequence bounds.
SELECT channel_id, event_seq FROM channel_event_head
 WHERE channel_id = :channel LIMIT 1;
SELECT channel_id, event_seq, event_type, msg_id, actor_id, create_at, update_at
  FROM channel_event
 WHERE channel_id = :channel AND event_seq BETWEEN :from_seq AND :to_seq
 ORDER BY event_seq LIMIT :limit;
SELECT id, channel_id, user_id, type, create_at, update_at FROM post
 WHERE channel_id = :channel ORDER BY create_at, id LIMIT :limit;
SELECT channel_id, user_id, team_id, role, update_at FROM channel_member
 WHERE channel_id = :channel AND team_id = :tenant ORDER BY user_id LIMIT :limit;
```

## Baseline Discovery (not a T050 candidate)

At `2026-07-21T21:36:00-07:00`, readonly MCP inspected local tenant key `444` and PG metadata. SQLite reported 9 tables (`channel`, `channel_member`, `message` among them) but no `channel_event_cursor`, `channel_event_ledger`, or `channel_sync_coverage`. PG reported all five required authority tables, and `channel_event` exposes `channel_id`, `event_seq`, `event_type`, `msg_id`, `actor_id`, `payload`, `create_at`, and `update_at`.

This establishes the required migration and query shapes only. It is **not** evidence of a recovery run. Current four-plane verdict: `BLOCKED` until an implemented candidate uses a current source pin and all required local tables exist in its isolated test database.

2026-07-21T21:36:00-07:00 | done: T001 source baseline and T002 bounded readonly evidence contract | current: current local tenant is pre-V2, four-plane evidence intentionally BLOCKED | next: freeze three-repository fixture naming/source-pin contract before foundational code
