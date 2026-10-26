# Data Model: Timeline V3 Delta Window

## TimelineEntityKey

Fields:

- `createAt: i64`: immutable server/client ordering timestamp.
- `temporaryId: string`: non-empty immutable client persistence identity.

Validation:

- Tuple uniqueness is scoped to a channel.
- Upsert cannot change either field.
- Empty, duplicate or conflicting keys reject the whole Frame.

## TimelineEntity

Fields:

- `key: TimelineEntityKey`
- complete renderable message value
- delivery/revoke/reaction/urgent/receipt business fields

Transitions:

- absent -> `UpsertMessage` plus `InsertKeys`
- sending -> sent/failed/recalled/updated through `UpsertMessage` on the same key
- no ordinary transition deletes or moves the key

## TimelineRange

Fields:

- `rangeId`
- `channelId`
- strictly ordered `keys`
- older/newer `RangeBound`
- persistence watermark/revision

Rules:

- A Range is internally contiguous for the facts already proved by persistence.
- `InsertKeys(start|end)` cannot introduce duplicates or reorder existing keys.
- `MergeRanges` requires adjacency/overlap without an unknown gap.
- Bounds and keys commit atomically.

## RangeBound

Fields:

- directional cursor
- `hasMore`
- persisted watermark that proved the bound

Rules:

- `hasMore=false` and a continuation cursor cannot conflict.
- A local lookahead item or server authoritative remaining count decides the bound and is not
  inserted into the requested window.

## TimelineSnapshot

Fields:

- epoch and revision
- channel-scoped entities
- one or more ranges with bounds
- active window reference
- persistence watermark

Use:

- initial attach, cold rebuild, explicit recovery and uncached latest rebuild only.

## TimelineDelta

Fields:

- epoch
- `baseRevision`
- result revision
- ordered closed operations:
  `UpsertMessage | InstallSegment | InsertKeys | MergeRanges | SetRangeBounds | ActivateWindow`

Rules:

- Validate every operation into a shadow state, then commit once.
- Any invalid key/range/bound/revision rejects all operations.
- Ordinary entity updates contain no Replace and no ActivateWindow.

## WindowSizePolicy

Fields:

- `requested?: integer`
- `effective: integer`
- constants `default=20`, `min=1`, `max=60`

Transitions:

- missing -> 20
- `1..=60` -> same value
- outside/non-integer -> rejected before I/O

## RenderWindow

Fields:

- active `rangeId`
- selected contiguous key interval of at most `pageSize`
- `WindowMode`
- visual anchor (consumer local)

Rules:

- References normalized entities; it does not own a second entity copy.
- Tail append at capacity drops the visible head locally.
- Local eviction does not create a Timeline operation or advance revision.

## WindowMode

Variants:

- `FollowLatest`
- `Anchored { anchorKey, pendingNewerCount }`
- `Located { targetKey, navigationToken, pendingNewerCount }`

Transitions:

- FollowLatest + scroll away -> Anchored
- Anchored/Located + latest append -> same mode, count +1
- explicit jump -> FollowLatest, count 0
- current locate token -> Located
- stale locate token -> cache-only, mode unchanged

## NavigationToken

Fields:

- opaque token
- channel/scope binding
- monotonic local intent generation

Rules:

- Only the current token may activate a located window.
- A stale response may install valid cache data but cannot change RenderWindow.

## TimelineCache

Fields:

- channel-indexed entity map
- Range map
- chunk/page LRU metadata
- entity-count and estimated-byte budgets
- pin set for visible, anchor, pending and in-flight data

Rules:

- Mutation work is limited to touched keys/ranges.
- Eviction works by bounded chunk, never scans all channels per Frame.
- Persistent SQLite remains the authority after cache eviction.

## GateFixtureManifest

Fields:

- Gate ID and test class
- producer revision and source path
- file inventory
- schema/surface versions and surface inventory
- raw/semantic SHA-256
- allowed parameter paths

States:

- generated -> validated Helix -> byte-synced PC -> byte-synced Mobile -> parity/hard-cut accepted
- any hash/inventory drift -> rejected
