# Contract: Timeline V3

## Window request

```text
Latest { pageSize? }
Older { before: EntityKey, pageSize? }
Newer { after: EntityKey, pageSize? }
Locate { target: EntityKey, pageSize?, navigationToken }
```

`pageSize` defaults to 20, accepts integer `1..=60`, and rejects other values before I/O.
The frontend cannot supply order, Range IDs, persistence cursors or rewritten EntityKeys.

## Locate allocation

```text
before = floor(pageSize / 2)
after  = pageSize - 1 - before
```

The target counts toward the total. A short side transfers its unused quota to the other side.
When both sides are sufficient, the target index is exactly `before`.

## Frame envelope

A Frame containing Timeline uses Frame schema 3 and declares Timeline surface schema 3. Other
surfaces in the same atomic Frame may retain their declared legacy surface schema.

Timeline payload is exactly one of:

- Snapshot: full normalized entity/range/window state for attach/recovery.
- Delta: epoch, base/result revision and an ordered list of closed operations.

## Delta operations

| Operation | Contract |
|---|---|
| `upsertMessage` | Insert or replace one complete entity value at an immutable EntityKey |
| `installSegment` | Install a new ordered Range and its bounds |
| `insertKeys` | Add unique keys at `start` or `end` of an existing Range |
| `mergeRanges` | Merge proved adjacent/overlapping Ranges |
| `setRangeBounds` | Atomically update directional bounds and watermark |
| `activateWindow` | Activate only an explicit current navigation intent |

There is no Replace, array-index patch, delete, move or trim operation.

## Required operation sequences

- New optimistic message: Upsert + edge Insert.
- Sent/failed/recalled/reaction/urgent/receipt update: one Upsert on the same key.
- Older/newer page: Upserts + edge Insert + necessary Bounds; no implicit Activate.
- Uncached locate: Upserts + InstallSegment + Activate(current token).
- Live append away from bottom: Upsert + tail Insert + necessary Bounds; no Activate.

## Atomic validation

Consumers validate schema, surface version, epoch, base revision, unique/stable keys, Range
continuity, operation references and bounds in shadow state. Only a fully valid Frame commits.
Failure retains the last correct state and requests one coalesced recovery Snapshot.
