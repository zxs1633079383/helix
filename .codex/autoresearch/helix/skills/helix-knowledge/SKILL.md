---
name: helix-knowledge
description: |
  Use for Helix repo questions or implementation planning involving runtime-agnostic core,
  sans-IO boundaries, helix-core/helix-im/driver-host/native/ffi architecture,
  HX-C harness constraints, sync-v2, Tauri seams, mobile FFI, golden replay,
  founder thesis, or maintainer review. Provenance: project repository distillation,
  unscored, current-at 2026-07-08.
---

# Helix Knowledge

## Quick Reference

Helix is a runtime-agnostic deterministic business-kernel base. The current product targets are PC Tauri/native and Flutter FFI. Web/wasm is archived/experimental unless a new ADR reactivates it.

Layer model:

- `helix-core`: runtime-neutral ports, `Module`, `Tick`, `Effect`, `ExecutionShell`; no I/O, no tokio, no business words.
- `helix-im`: first business module; owns channel/message/cursor/readBits/sync/outbound/projection semantics.
- `helix-driver-host`: shared host I/O for native + FFI: SQLite, HTTP/WS, `BatchSink`, `RecordingSink`, trace hooks, local store, bounded spawner.
- `helix-driver-native`: PC/Tauri shell over host driver.
- `helix-driver-ffi`: mobile C ABI shell over host driver.

## Maintainer Invariants

- Core never imports runtime/platform crates and never owns concurrency.
- Driver -> core only; core never depends on driver.
- Business schema and wire domain words stay in modules, not `helix-core`.
- Time, IDs, storage, network, and event emission are injected by host/driver.
- FFI: no unwind across C ABI, exact raw pointer ownership, batch event/effect crossing.
- Hot path: O(1), no accidental allocation, no read-modify-write in critical writes.
- Ordered effects must be deterministic.
- Memory is routing context only; verify repo state locally before claiming current truth.

## Current Live Work

Check `docs/ROADMAP.md` before acting. Current open items include Tauri four seams in Loopforge, golden frame export/replay, feature split debt, multi-module concurrent write proof, any live `structure-gate` red items, and any `review-gap` red state. AI engineering gates are closed in repo docs; local public-api diff still depends on the `cargo-public-api` tool being installed.

## Verification Commands

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-core
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
bash scripts/structure-gate.sh
bash scripts/trace-static-gate.sh
```

Run `scripts/gate.sh` for default pre-commit behavioral verification when code changed.

## Agent Rules

- Start with `docs/GOAL.md`, `docs/ROADMAP.md`, and `docs/harness/OPERATING.md`.
- Prefer code graph tools for symbol discovery, shell search for docs/config/literals.
- Do not touch `crates/**` for knowledge-only tasks.
- If changing same-directory `AGENTS.md` or `CLAUDE.md`, mirror both exactly.
- Ask for human review on ABI, storage schema, public command/event schema, and cross-repo cutover changes.

For exhaustive detail, see `../../knowledge/helix_knowledge.md`.
