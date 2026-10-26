# coverage.sh MATCH_KEY Review After helix-im DDD Split

## Scope

Reviewed `scripts/coverage.sh --phase <file>` after the #9-#18 DDD directory split.

## Code Path

`MATCH_KEY="${FILE#crates/helix-im/src/}"` strips only the crate source prefix and preserves deeper directory components below `src/`.

The resulting key is passed to `grep -F "$MATCH_KEY"` against `cargo llvm-cov --summary-only` output. That means:

- `crates/helix-im/src/channel/gate.rs` becomes `channel/gate.rs`
- `crates/helix-im/src/ws/parser/core.rs` becomes `ws/parser/core.rs`
- `crates/helix-im/src/outbound/user_misc/core.rs` becomes `outbound/user_misc/core.rs`

## 目录变深影响

The DDD split makes paths deeper, but this does not require a `coverage.sh` code change. The current prefix-strip behavior keeps the relative path below `crates/helix-im/src/`, which is exactly the stable identity needed when files move into nested bounded-context folders.

Changing the script to strip only a fixed number of path segments, or to match by basename, would be weaker because duplicate basenames such as `core.rs`, `mod.rs`, and `read.rs` now exist in multiple subdomains.

## 结论

No `coverage.sh` code change is required.

The existing `MATCH_KEY="${FILE#crates/helix-im/src/}"` expression remains semantically correct after the DDD split because it is depth-agnostic and preserves the nested crate-local path used for matching.

## Verification

- RED: `scripts/ddd-structure-gate.sh 20` failed before this document existed.
- Static review: inspected `scripts/coverage.sh --phase` and confirmed `MATCH_KEY` preserves nested paths.
- Runtime check skipped: `cargo llvm-cov --version` failed locally with `error: no such command: llvm-cov`, so `scripts/coverage.sh --phase crates/helix-im/src/ws/parser/core.rs` could not be run in this environment.
