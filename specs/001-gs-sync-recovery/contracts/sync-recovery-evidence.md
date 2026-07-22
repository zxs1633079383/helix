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
