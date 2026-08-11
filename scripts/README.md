# Helix Scripts

This directory keeps only durable entrypoints. One-off UC probes should live in
docs, workflow notes, or `host-cli-session.sh` command snippets, not as new
top-level scripts.

## Core Gates

- `gate-fast.sh` - scope-aware local/pre-push gate; scripts and low-risk crates run only relevant checks.
- `gate.sh` - full CI/merge delivery gate for native + FFI.
- `branch-identity.sh` - branch identity report for SESSION handoff.
- `branch-sync-gate.sh` - branch/upstream drift guard.
- `ddd-structure-gate.sh` - DDD/package boundary structure gate.
- `coverage.sh` - Rust coverage entrypoint.
- `fullmap-coverage.sh` - IM migration coverage table check.
- `night-loop-preflight.sh` - overnight loop readiness preflight.

## Runtime And FFI

- `dev-loop.sh` - canonical local dev loop for host-cli against Go.
- `host-cli-session.sh` - canonical persistent host-cli session helper.
- `host-cli-FULL-uc-e2e.sh` - canonical full UC e2e sweep.
- `host-cli-dual-account-e2e.sh` - dual-account/broadcast e2e scenarios.
- `messagev3/phase1-real-chain.mjs` - Phase1 `Domain → Gate` three-repo runner; internally targets Go `127.0.0.1:8066`, invokes Angular real WDIO classes, and fails closed when HTTP/WS/Native/DOM/storage evidence is incomplete.
- `ffi-verify.sh` - Flutter/FFI export and boundary validation.
- `cpp-smoke.sh` - C++ FFI smoke check.

## Setup And Shared Config

- `install-hooks.sh` - install repository hooks.
- `lib-config.sh` - shared shell config loader for scripts.

## Entropy Rule

Do not add a new `host-cli-uc*.sh` for a single bug or historical probe. Add the
scenario to `host-cli-FULL-uc-e2e.sh` if it is a durable regression, or document
the command sequence under `docs/harness/` / `docs/review/` if it is evidence.
