# module.rs Structure Gate Design

## Goal

Reduce `crates/helix-im/src/module.rs` from the current 1316-line whitelist debt by moving existing responsibilities into focused files, without changing IM behavior or relaxing `structure-gate`.

## Context

Required project documents were read: `AGENTS.md`, `docs/GOAL.md`, `.claude/rules/code-structure-ddd.md`, `scripts/structure-gate.sh`, and recent `docs/review/workflow-review-ledger.md` structure-gate entries. GitNexus status is up-to-date at commit `287d3e2`.

The structure rule is strict: `src/**.rs` files should be <=300 lines, whitelist baselines are red lines, and baseline changes may only move downward. Recent review history repeatedly called out baseline increases as a governance failure, so this task must reduce the debt rather than hide it.

## Design

Use narrow, behavior-preserving extraction. Keep `ImConfig`, `ImModule`, allocator helpers, public inspection helpers, `dispatch_ws_frame`, `read_req_id`, and the `Module` trait implementation in `module.rs`. Move method bodies into sibling modules that each add an `impl ImModule` block:

- `send_flow.rs`: `handle_send_message`.
- `query_dispatch.rs`: `handle_query_command`.
- `sync_flow.rs`: `handle_scan_reply`, `handle_hello`, proactive resync helpers, `maybe_continue_sync`, `handle_sync_reply`, and `drain_sync_queue`.
- `port_reply.rs`: `handle_port_reply` and correlation-context reply arms.
- `lifecycle.rs`: lifecycle/timer/control helpers if needed to keep file sizes under 300.

No new trait layer, registry, macro, or business abstraction is introduced. The extraction may require changing helper visibility from private to `pub(crate)` within `ImModule`, but method signatures and effect order must remain equivalent.

## Risk

GitNexus impact marks `handle_send_message`, `handle_port_reply`, `handle_sync_reply`, `maybe_continue_sync`, and `handle_query_command` as `CRITICAL` because they are on the main `Module::handle` execution chain and affect module tests and benches. The mitigation is pure movement with no signature or branch-order changes, plus targeted structural review and the existing `helix-im`/`helix-core` test suites.

The local worktree already has pre-existing uncommitted `AGENTS.md`, `CLAUDE.md`, and `outputs/` changes. This task must not revert or stage them unless explicitly asked.

## Verification

Required verification:

- `./scripts/structure-gate.sh`
- `cargo test -p helix-im`
- `cargo test -p helix-core`
- `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` if needed after import or visibility changes

Before commit, run GitNexus `detect_changes` and verify affected symbols match the expected module extraction scope.

