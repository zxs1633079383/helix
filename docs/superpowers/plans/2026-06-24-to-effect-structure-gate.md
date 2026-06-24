# to_effect.rs Structure Gate Increment Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove `crates/helix-im/src/acl/to_effect.rs` from structure-gate whitelist by splitting real Effect factory responsibilities while preserving the `acl::to_effect::*` API.

**Architecture:** Keep `to_effect.rs` as a compatibility facade that re-exports focused sibling modules. Split by Effect family: transport frame construction, storage effect factories, sync HTTP request factories, message-row projection factories, and control/thin projection factories.

**Tech Stack:** Rust workspace, `helix-im`, `helix-core::Effect`, `scripts/structure-gate.sh`, projection schema tests.

## Global Constraints

- Do not raise `structure-gate` baseline.
- Do not add new large files to the whitelist.
- Do not loosen `scripts/structure-gate.sh`.
- Only reduce whitelist file line counts, or remove a file from whitelist once it is <=300.
- Preserve `helix-core` zero-runtime boundary and do not modify `crates/helix-core/**`.
- Preserve IM business semantics, event names, payload field sets, and effect order.
- Preserve existing call path compatibility through `crate::acl::to_effect::*`.
- Every new `crates/helix-im/src/acl/*.rs` file must be <=300 lines.
- Do not create a new source directory, avoiding new CLAUDE.md/AGENTS.md mirror obligations.

---

## File Structure

- Modify: `crates/helix-im/src/acl/mod.rs` to declare new sibling modules.
- Modify: `crates/helix-im/src/acl/to_effect.rs` to become a facade re-exporting the split modules and existing `sync_effects`.
- Create: `crates/helix-im/src/acl/transport_effects.rs` for `ping_frame`.
- Create: `crates/helix-im/src/acl/storage_effects.rs` for message/channel/cursor storage factories.
- Create: `crates/helix-im/src/acl/sync_http_effects.rs` for sync/increment HTTP factories and `connection_id_headers`.
- Create: `crates/helix-im/src/acl/projection_message_effects.rs` for `message_item_data` and message-row fat emit factories.
- Create: `crates/helix-im/src/acl/projection_control_effects.rs` for thin/control/pass-through emit factories.
- Modify: `scripts/structure-gate.sh` to remove `to_effect.rs` from WHITELIST after it is <=300.
- Modify: `.claude/rules/code-structure-ddd.md` to mirror the script baseline truth and split note.

### Task 1: Transport and Storage Factories

**Files:**
- Create: `crates/helix-im/src/acl/transport_effects.rs`
- Create: `crates/helix-im/src/acl/storage_effects.rs`
- Modify: `crates/helix-im/src/acl/to_effect.rs`
- Modify: `crates/helix-im/src/acl/mod.rs`

- [ ] Move `ping_frame` into `transport_effects.rs`.
- [ ] Move `upsert_message`, `upsert_channel_full`, `update_channel_partial`, `bump_channel_unread`, `bump_channel_unread_op`, and `advance_cursor` into `storage_effects.rs`.
- [ ] Re-export these functions from `to_effect.rs` so existing callers compile unchanged.
- [ ] Verify `wc -l` for new files is <=300.

### Task 2: Sync HTTP Factories

**Files:**
- Create: `crates/helix-im/src/acl/sync_http_effects.rs`
- Modify: `crates/helix-im/src/acl/to_effect.rs`
- Modify: `crates/helix-im/src/acl/mod.rs`

- [ ] Move `sync_notify`, `increment_http_trigger`, and `connection_id_headers` into `sync_http_effects.rs`.
- [ ] Re-export them from `to_effect.rs`.
- [ ] Keep HTTP method, URL, headers, and JSON body shapes byte-equivalent.

### Task 3: Projection Factories

**Files:**
- Create: `crates/helix-im/src/acl/projection_message_effects.rs`
- Create: `crates/helix-im/src/acl/projection_control_effects.rs`
- Modify: `crates/helix-im/src/acl/to_effect.rs`
- Modify: `crates/helix-im/src/acl/mod.rs`

- [ ] Move `message_item_data` plus fat message-row emits into `projection_message_effects.rs`.
- [ ] Move thin/control/pass-through emits into `projection_control_effects.rs`.
- [ ] Re-export all public emit factories from `to_effect.rs`.
- [ ] Run `cargo test -p helix-im --test projection_schema_test` and `cargo test -p helix-im --test uc4_sync_increment_test`.

### Task 4: Whitelist and Verification

**Files:**
- Modify: `scripts/structure-gate.sh`
- Modify: `.claude/rules/code-structure-ddd.md`
- Modify: `SESSION.md`

- [ ] Confirm `crates/helix-im/src/acl/to_effect.rs <=300`.
- [ ] Remove `crates/helix-im/src/acl/to_effect.rs` from `scripts/structure-gate.sh` WHITELIST.
- [ ] Update `.claude/rules/code-structure-ddd.md` so the baseline table matches the script true source and mentions the new split.
- [ ] Run `./scripts/structure-gate.sh`.
- [ ] Run `cargo test -p helix-im`.
- [ ] Run `cargo test -p helix-core`.
- [ ] Run `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi`.
- [ ] Run GitNexus `detect_changes(scope: "all", repo: "helix")`.
