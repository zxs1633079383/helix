# G-S1 Cold-start Closure Evidence

## PC

- Single: `gs1-cold-single-20260727030223`, exit 0, channel
  `h3r1ns8a6byspmw8nq3raj813h`, Actor 444 hash
  `7ac1b3ddf45cec2f4c6d16c6fc3950094d9fa6d6b40d99466193f5c960eaf178`,
  missing/extra/mismatch = 0/0/0.
- Dual: `gs1-cold-dual-20260727032800`, exit 0, channel
  `tjhu7z453prz8jfytjrw1szayo`, Actor 444 and Observer 678 hash
  `ac84fafc50b44658e6141ce7d54cb1c19160f5ce91dbc188add977bd2af39622`,
  missing/extra/mismatch = 0/0/0 for both personas.
- Reset: app processes stopped before deletion; main/WAL/SHM deleted from the
  same actor and observer profile roots; residual artifacts = 0;
  `remoteDeletionPerformed=false`.
- Projection: seed and recover WDIO reports passed; Actor/Observer dialog,
  FrameApplied and DOM converged.

## Mobile

- Run: `mrc-mobile-uc-20260727104758-27652`, GS01 8/8 PASS in 16.367s,
  channel `6ebp7ypfrigozjrqsq479aqoxw`.
- userA and userB each disposed the production runtime, deleted their own
  SQLite main/WAL/SHM, and restarted on the same database path.
- Both personas produced remote/local hash
  `4c4eaef40c36043de841ab9722b7d89ee7ee9eba44baeae336530bee191ad87d`
  with missing/extra/mismatch = 0/0/0.
- Reset residual artifacts = 0 and `remoteDeletionPerformed=false` for both
  personas.
- Projection: typed dialog and timeline Frames were atomically applied and
  exposed through the QuickJS typed view after cold recovery.

## Boundaries

- PostgreSQL is read-only evidence authority; no remote deletion occurred.
- Reset permits only registered SQLite main/WAL/SHM/journal artifacts and
  rejects path escape, links, active locks and residual files.
- Mock, HTTP, WS or Frame evidence alone was not used as storage parity.
