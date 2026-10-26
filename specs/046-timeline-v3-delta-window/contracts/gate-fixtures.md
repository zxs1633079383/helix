# Contract: Formal Gate Fixtures

## Gate inventories

### G-10b LoadNewerGate

- `attach-empty.snapshot.frame-v3.json`
- `latest-loaded.delta.frame-v3.json`
- `initial-located.delta.frame-v3.json`
- `newer-loaded.delta.frame-v3.json`
- `newer-exhausted.delta.frame-v3.json`
- `newer-late.delta.frame-v3.json`
- `provenance.json`

### G-10c FullWindowAppendEvictionGate

- `attach-empty.snapshot.frame-v3.json`
- `full-window-loaded.delta.frame-v3.json`
- `new-message-appended.delta.frame-v3.json`
- `provenance.json`

Head eviction is asserted in PC/Mobile normalized window state and is not a producer operation.

### G-10d AnchoredNewMessageGate

- `attach-empty.snapshot.frame-v3.json`
- `anchored-loaded.delta.frame-v3.json`
- `anchored-new-message.delta.frame-v3.json`
- `provenance.json`

The producer Delta contains no ActivateWindow. Jump-to-latest is a consumer action/state assertion.

### G-11a UncachedLocateBidirectionalGate

- `attach-empty.snapshot.frame-v3.json`
- `latest-loaded.delta.frame-v3.json`
- `uncached-located.delta.frame-v3.json`
- `located-older.delta.frame-v3.json`
- `located-newer.delta.frame-v3.json`
- `stale-navigation.delta.frame-v3.json`
- `provenance.json`

## Required provenance fields

- Gate ID and test class
- Helix producer revision and source path
- per-file identity and byte length
- raw SHA-256
- semantic SHA-256
- Frame and Timeline surface schema versions
- complete surface inventory
- allowed parameter paths

Semantic hashing includes operation kind/order, EntityKey, Range insertion/merge position, Bounds,
watermark, epoch/base/result revision and active window. Final rendered values alone are
insufficient.

The first Frame in each formal inventory is the native attach Snapshot at revision 1. Loading the
configured latest window is a Delta at revision 2, so replay matches the production Tauri lifecycle
instead of manufacturing a second revision-1 Snapshot.

## Parameter matrix

Every formal Gate runs with `pageSize=20` and 60. Relevant request tests also reject 0 and 61.
Locate verifies exact target index and short-side refill.

## Promotion order

```text
13 existing Helix Timeline Gates
-> G-10b/G-10c/G-10d/G-11a Helix producer and canonical assets
-> cses-client byte sync and WDIO
-> Mobile byte sync and GTest
-> cross-repository semantic parity and Timeline V2 hard-cut
```
