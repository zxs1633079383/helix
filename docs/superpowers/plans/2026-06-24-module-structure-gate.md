# module.rs Structure Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce `crates/helix-im/src/module.rs` whitelist debt by extracting existing responsibilities into focused files while preserving behavior.

**Architecture:** Keep `module.rs` as the `ImModule` shell and `Module` trait entrypoint. Move existing inherent methods into sibling `impl ImModule` modules with no new abstraction layer, no behavior rewrites, and no effect-order changes.

**Tech Stack:** Rust workspace, `helix-im`, `helix-core`, GitNexus, `scripts/structure-gate.sh`, Cargo test/check.

## Global Constraints

- Do not raise `structure-gate` baseline.
- Do not add new large files to the whitelist.
- Do not loosen `scripts/structure-gate.sh`.
- Only reduce whitelist file line counts, or remove a file from whitelist once it is <=300 lines.
- Protect existing uncommitted `AGENTS.md`, `CLAUDE.md`, and `outputs/` changes.
- Do not modify `crates/helix-core/**`.
- Preserve IM business semantics and effect order.
- Every new `crates/helix-im/src/*.rs` file must be <=300 lines.
- Cargo discipline: do not run cargo after every small move; batch type fixes, then verify.

---

## File Structure

- Modify: `crates/helix-im/src/lib.rs` to declare new modules.
- Modify: `crates/helix-im/src/module.rs` to remove moved method bodies and keep `ImConfig`, `ImModule`, allocators, `dispatch_ws_frame`, `read_req_id`, `Module` implementation, and tests module hook.
- Create: `crates/helix-im/src/send_flow.rs` for `handle_send_message`.
- Create: `crates/helix-im/src/query_dispatch.rs` for `handle_query_command`.
- Create: `crates/helix-im/src/sync_flow.rs` for scan/hello/proactive sync/sync reply/drain methods.
- Create: `crates/helix-im/src/port_reply.rs` for `handle_port_reply`.
- Create: `crates/helix-im/src/lifecycle.rs` only if `module.rs` remains too large after the first four extractions.
- Modify: `scripts/structure-gate.sh` to lower `module.rs` baseline to the post-refactor line count, or remove the whitelist entry if it becomes <=300.
- Modify: `.claude/rules/code-structure-ddd.md` to mirror the lowered baseline and update the split note.
- Modify: `SESSION.md` with final `DONE` after verification.

### Task 1: Extract Send Flow

**Files:**
- Create: `crates/helix-im/src/send_flow.rs`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/module.rs`

**Interfaces:**
- Consumes: `ImModule::alloc_corr_internal(&mut self) -> helix_core::Correlation`, `ImModule::alloc_timer(&mut self) -> TimerId`, `ImConfig::user_identity(&self)`.
- Produces: `pub(crate) fn handle_send_message(&mut self, payload: &[u8], now_ms: u64, out: &mut EffectSink) -> Result<(), ImError>`.

- [ ] Move the existing `handle_send_message` body verbatim into `send_flow.rs`.
- [ ] Add imports needed by that body: `crate::error::ImError`, `crate::module::ImModule`, `crate::pending_send::PendingSend`, `crate::state::{ChannelId, SendStatus, TemporaryId}`, `helix_core::effect::HttpRequest`, `helix_core::{Effect, EffectSink}`.
- [ ] Add `mod send_flow;` in `lib.rs`.
- [ ] Change `alloc_corr_internal` visibility to `pub(crate)` because sibling modules need it.
- [ ] Check line count: `wc -l crates/helix-im/src/send_flow.rs crates/helix-im/src/module.rs`; expected `send_flow.rs <=300`.

### Task 2: Extract Query Dispatch

**Files:**
- Create: `crates/helix-im/src/query_dispatch.rs`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/module.rs`

**Interfaces:**
- Consumes: `ImModule::alloc_corr_internal`, `self.config.api_base_url`, `self.state.connection_id`, `self.state.corr_map`.
- Produces: `pub(crate) fn handle_query_command(&mut self, name: &str, payload: &[u8], out: &mut EffectSink) -> Result<(), ImError>`.

- [ ] Move the existing `handle_query_command` body verbatim into `query_dispatch.rs`.
- [ ] Keep query command names and match order unchanged.
- [ ] Add `mod query_dispatch;` in `lib.rs`.
- [ ] Check line count: `wc -l crates/helix-im/src/query_dispatch.rs`; expected `<=300`.

### Task 3: Extract Sync Flow

**Files:**
- Create: `crates/helix-im/src/sync_flow.rs`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/module.rs`

**Interfaces:**
- Consumes: `ImModule::alloc_corr_internal`, `self.config`, `self.state`, `ChannelId`, `CorrelationContext`, `Seq`.
- Produces: `handle_scan_reply`, `handle_hello`, `emit_proactive_resync`, `emit_proactive_resync_for`, `maybe_continue_sync`, `handle_sync_reply`, `drain_sync_queue`.

- [ ] Move these methods as a group to preserve their internal call relationships.
- [ ] Keep deterministic sorting in `emit_proactive_resync` unchanged.
- [ ] Keep `SyncResponse` match behavior and `wants_continuation` registration unchanged.
- [ ] Add `mod sync_flow;` in `lib.rs`.
- [ ] Check line count: `wc -l crates/helix-im/src/sync_flow.rs`; expected `<=300`. If it exceeds 300, split scan/hello helpers into `lifecycle.rs`.

### Task 4: Extract Port Reply

**Files:**
- Create: `crates/helix-im/src/port_reply.rs`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/module.rs`

**Interfaces:**
- Consumes: sync/query/send helper methods from earlier tasks.
- Produces: `pub(crate) fn handle_port_reply(&mut self, corr: helix_core::Correlation, outcome: &helix_core::tick::PortOutcome, now_ms: u64, out: &mut EffectSink) -> Result<(), ImError>`.

- [ ] Move the existing `handle_port_reply` body verbatim into `port_reply.rs`.
- [ ] Preserve `corr_map.remove(&corr)` before match.
- [ ] Preserve all `CorrelationContext` arms and no-op unknown corr behavior.
- [ ] Add `mod port_reply;` in `lib.rs`.
- [ ] Check line count: `wc -l crates/helix-im/src/port_reply.rs`; expected `<=300`. If it exceeds 300, split only reply helper arms into small private functions in the same file or move query/read/todo reply helpers into `query_dispatch.rs`.

### Task 5: Optional Lifecycle Split

**Files:**
- Create: `crates/helix-im/src/lifecycle.rs` if needed.
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/module.rs`

**Interfaces:**
- Produces moved lifecycle/timer helpers only if needed to bring `module.rs` lower without creating a >300 file.

- [ ] If `module.rs` is still above the desired lowered baseline, move `handle_timer`, `on_start`, and `on_stop` support code into `lifecycle.rs` while keeping the `Module` trait methods in `module.rs` as thin delegators if that is simpler.
- [ ] Preserve timer cancellation order.
- [ ] Check `lifecycle.rs <=300`.

### Task 6: Structure Gate Baseline and Docs

**Files:**
- Modify: `scripts/structure-gate.sh`
- Modify: `.claude/rules/code-structure-ddd.md`

**Interfaces:**
- Produces updated `module.rs` baseline that is strictly less than 1318, or removes the whitelist entry if `module.rs <=300`.

- [ ] Run `wc -l crates/helix-im/src/module.rs` and record the new line count.
- [ ] Lower the `crates/helix-im/src/module.rs` whitelist number in `scripts/structure-gate.sh` to that exact count if still >300.
- [ ] Mirror the same number and split explanation in `.claude/rules/code-structure-ddd.md`.
- [ ] Do not change other whitelist numbers unless this task directly reduced those files.

### Task 7: Verification and Change Detection

**Files:**
- Modify: `SESSION.md`

**Interfaces:**
- Produces verification evidence and final DONE record.

- [ ] Run `./scripts/structure-gate.sh`; expected `STRUCT-GATE ✅ PASS`.
- [ ] Run `cargo test -p helix-im`; expected all tests pass.
- [ ] Run `cargo test -p helix-core`; expected all tests pass.
- [ ] If import/visibility changes touched cross-crate compilation assumptions, run `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi`.
- [ ] Run GitNexus `detect_changes(scope: "all", repo: "helix")` and confirm affected scope is the expected module extraction.
- [ ] Append `✅ DONE module.rs 结构闸白名单治理 ...` to `SESSION.md`, including old/new line counts, created files, gate/test results, and whitelist status.

## Self-Review

Spec coverage: every required constraint maps to Tasks 1-7. Placeholder scan: no task uses TBD/TODO/implement later language. Type consistency: helper names match existing `ImModule` methods and preserve the original signatures.

