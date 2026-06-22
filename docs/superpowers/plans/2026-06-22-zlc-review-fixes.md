# ZLC Review Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the `fa07ff1..HEAD` ZLC review follow-up list without weakening Helix's runtime-agnostic core boundary.

**Architecture:** Keep business ID fixtures inside `helix-im` tests or `#[cfg(test)]` helpers, keep production ID parsing strict, and make hygiene gates catch whitespace regressions. Treat `StorageOp::BatchDelete` as a core/public-port change spanning host/native/FFI/archived wasm documentation, not as a local IM-only change.

**Tech Stack:** Rust 2021 workspace, `helix-core`, `helix-im`, `helix-driver-host`, `helix-driver-native`, `helix-driver-ffi`, archived `helix-driver-wasm`, shell gate scripts.

## Global Constraints

- Use Chinese for controller-facing reports.
- Do not expose test-only ID constructors in production public API.
- `Id26::from_str` accepts exactly 26 ASCII lowercase alphanumeric bytes: `[a-z0-9]`.
- Determinism tests must use fixed 26-character fixture IDs; do not rely on synthetic decimal zero-padding sort behavior.
- `helix-core` remains runtime-agnostic and must not gain `tokio`, `sqlx`, `reqwest`, `web-sys`, or `tauri`.
- Productized targets are PC Tauri/native and Flutter FFI. `helix-driver-wasm` is archived and must not become a required delivery target.
- Preserve unrelated dirty worktree changes; do not revert user or IDE changes.
- For symbol edits, run GitNexus impact first and run `detect_changes()` before commit.
- Each task must have an implementation report and a task-scoped reviewer gate before moving on.

---

## File Structure

- Modify `crates/helix-im/src/state.rs`
  - Own strict `Id26` parsing and test-only `#[cfg(test)]` fixture helpers.
- Modify `crates/helix-im/tests/common.rs`
  - Own integration-test channel/server fixture constructors.
- Modify selected `crates/helix-im/tests/*.rs` and `crates/helix-im/benches/*.rs`
  - Replace removed public test constructors and numeric-sort assumptions.
- Modify selected driver tests under `crates/helix-driver-host/tests/` and `crates/helix-driver-ffi/tests/`
  - Use real-shape fixture IDs through local test helpers.
- Modify docs under `docs/harness/` and storage design docs
  - Keep ADRs and archived wasm boundary aligned with current behavior.
- Modify `scripts/gate.sh`
  - Add whitespace checking.
- Inspect or update storage public API files:
  - `crates/helix-core/src/effect.rs`
  - `crates/helix-core/src/ports/storage.rs`
  - `crates/helix-driver-host/src/storage.rs`
  - `crates/helix-driver-native/src/storage.rs`
  - `crates/helix-driver-ffi/src/**`
  - `crates/helix-driver-wasm/src/storage.rs`

### Task 1: Strict ID Parsing and Test Fixture Isolation

**Files:**
- Modify: `crates/helix-im/src/state.rs`
- Modify: `crates/helix-im/tests/common.rs`
- Modify: `crates/helix-im/tests/s6_sync_internal_test.rs`
- Modify: selected `crates/helix-im/tests/*.rs`
- Modify: selected `crates/helix-im/benches/*.rs`
- Modify: `crates/helix-driver-host/tests/im_event_persist.rs`
- Modify: `crates/helix-driver-ffi/tests/f5_egress_parity.rs`
- Modify: `docs/harness/decisions.md`

**Interfaces:**
- Consumes: existing `Id26::from_str(s: &str) -> Result<Id26, ImError>`.
- Produces: strict production ID parser and test-local fixture constructors that return public `ChannelId` / `ServerId` values without public test bridge APIs.

- [ ] **Step 1: Write the failing alphabet test**

Add this test in `crates/helix-im/src/state.rs`:

```rust
#[test]
fn from_str_rejects_invalid_ascii_alphabet() {
    for value in [
        "5ffd465e263864787b87aa012!",
        "5ffd465e263864787b87aa012 ",
        "5ffd465e263864787b87aa012_",
        "5ffd465e263864787b87aa012A",
    ] {
        assert!(Id26::from_str(value).is_err(), "{value}");
    }
}
```

- [ ] **Step 2: Run RED**

Run: `cargo test -p helix-im id26_tests::from_str_rejects_invalid_ascii_alphabet`

Expected before implementation: FAIL because punctuation, space, underscore, or uppercase ASCII are accepted.

- [ ] **Step 3: Implement strict parser**

Update `Id26::from_str` to reject any byte outside lowercase ASCII letters and digits:

```rust
let valid = s
    .as_bytes()
    .iter()
    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
if !valid {
    return Err(ImError::Parse("expected 26-char [a-z0-9] id".into()));
}
```

- [ ] **Step 4: Remove production public test constructors**

Remove these production public APIs from `crates/helix-im/src/state.rs`:

```rust
pub fn from_test_u64(n: u64) -> Self
```

for `Id26`, `ChannelId`, and `ServerId`.

- [ ] **Step 5: Add test-only fixture helpers**

Use `#[cfg(test)]` helpers in `state.rs` for unit tests and `tests/common.rs` helpers for integration tests. Fixture strings must be exactly 26 chars and match `[a-z0-9]`, for example:

```rust
format!("chfixx{n:020x}")
format!("srvfix{n:020x}")
```

- [ ] **Step 6: Remove synthetic ordering dependency**

In `crates/helix-im/tests/s6_sync_internal_test.rs`, use fixed real-shape IDs whose lexicographic order is obvious from alphabetic prefixes:

```rust
const CH_LOW: &str = "aaaaaa00000000000000000002";
const CH_MID: &str = "mmmmmm00000000000000000011";
const CH_HIGH: &str = "zzzzzz0000000000000000001e";
```

- [ ] **Step 7: Verify Task 1**

Run:

```bash
cargo test -p helix-im
cargo test -p helix-driver-host --test im_event_persist
cargo test -p helix-driver-ffi --test f5_egress_parity
cargo check -p helix-im --benches
rg -n 'from_test_u64' crates docs --glob '!SESSION.md' --glob '!docs/superpowers/plans/2026-06-22-zlc-review-fixes.md'
git diff --check
```

Expected:
- all cargo commands pass;
- `rg` returns no matches;
- `git diff --check` reports no whitespace errors.

- [ ] **Step 8: Request task review**

Generate a review package from the base commit before Task 1 to current HEAD or current working diff, then dispatch a task reviewer. Critical and Important findings must be fixed and re-reviewed before Task 2 starts.

### Task 2: Whitespace Hygiene and Gate Enforcement

**Files:**
- Modify: `scripts/gate.sh`
- Modify: `crates/helix-driver-ffi/docs/design/cpp-direct/00-设计思想.md`
- Modify: `crates/helix-driver-host/docs/design/00-设计思想.md`
- Modify: `crates/helix-driver-native/docs/design/00-设计思想.md`
- Modify: `crates/helix-driver-host/docs/design/07-storage-rw-concurrency-spec.md`

**Interfaces:**
- Consumes: current review baseline `fa07ff1..HEAD`.
- Produces: a gate step that fails on whitespace errors and a diff range that passes whitespace checks.

- [ ] **Step 1: Confirm current whitespace failures**

Run:

```bash
git diff --check fa07ff1..HEAD
```

Expected before fixes: whitespace errors in the known docs files or any newly surfaced file.

- [ ] **Step 2: Fix whitespace only**

Edit only the flagged trailing-whitespace or blank-line whitespace. Do not rewrite prose and do not reformat unrelated markdown.

- [ ] **Step 3: Add whitespace gate**

In `scripts/gate.sh`, add a dedicated step that runs `git diff --check`. If the script already has step helpers, use the existing helper style. If it needs a range-aware mode, default to working tree/current staged diff and document the command in the log line.

- [ ] **Step 4: Verify Task 2**

Run:

```bash
git diff --check fa07ff1..HEAD
scripts/gate.sh
```

Expected:
- the baseline range whitespace check passes;
- `scripts/gate.sh` includes and passes the whitespace step, or fails later for a clearly unrelated pre-existing gate failure that is reported.

- [ ] **Step 5: Request task review**

Generate a Task 2 review package and dispatch a task reviewer. Critical and Important findings must be fixed and re-reviewed before Task 3 starts.

### Task 3: `StorageOp::BatchDelete` Cross-End Coverage and Public-Port Documentation

**Files:**
- Inspect/modify: `crates/helix-core/src/effect.rs`
- Inspect/modify: `crates/helix-core/src/ports/storage.rs`
- Inspect/modify: `crates/helix-driver-host/src/storage.rs`
- Inspect/modify: `crates/helix-driver-native/src/storage.rs`
- Inspect/modify: `crates/helix-driver-ffi/src/**`
- Inspect/modify: `crates/helix-driver-wasm/src/storage.rs`
- Inspect/modify: related docs or ADRs under `docs/harness/`

**Interfaces:**
- Consumes: existing `StorageOp::BatchDelete` and `Storage::batch_delete` API.
- Produces: explicit test/doc evidence that host/native/FFI behavior is covered and archived wasm handling is intentional.

- [ ] **Step 1: Run impact and locate callers**

Run GitNexus impact on the storage symbols if available, then inspect direct text references:

```bash
rg -n 'BatchDelete|batch_delete' crates docs scripts
```

Expected: complete list of core definitions, driver implementations, tests, and docs.

- [ ] **Step 2: Confirm host/native/FFI tests**

Run or add focused tests so the shared host storage path proves batch delete behavior. Native and FFI can rely on shared host-driver behavior only if docs or tests make that delegation explicit.

- [ ] **Step 3: Clarify archived wasm strategy**

If `helix-driver-wasm` remains compiling with a stub implementation, document that it follows trait shape only because the crate exists, not because wasm is in delivery scope. Do not add wasm to required CI or roadmap.

- [ ] **Step 4: Update public-port docs**

Update `crates/helix-core/src/ports/storage.rs` comments so they describe current boundaries:

```text
helix-driver-host provides the shared rusqlite-backed storage implementation used by PC Tauri/native and Flutter FFI delivery paths.
helix-driver-wasm is archived/experimental and not part of required delivery validation.
```

- [ ] **Step 5: Verify Task 3**

Run:

```bash
cargo test -p helix-core -p helix-driver-host -p helix-driver-native -p helix-driver-ffi
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

Expected:
- cargo test/check pass;
- the `cargo tree | grep` command returns no matches.

- [ ] **Step 6: Request task review**

Generate a Task 3 review package and dispatch a task reviewer. Critical and Important findings must be fixed and re-reviewed before Task 4 starts.

### Task 4: Remaining P2/P3 Audit and Gate Alignment

**Files:**
- Inspect/modify: `crates/helix-driver-wasm/src/storage.rs`
- Inspect/modify: `docs/harness/decisions.md`
- Inspect/modify: `crates/helix-host-cli/src/main.rs`
- Inspect/modify: `scripts/ffi-verify.sh`
- Inspect/modify: `scripts/gate.sh`
- Inspect/modify: `crates/helix-im/tests/common.rs`
- Inspect/modify: docs describing FFI verification trigger conditions

**Interfaces:**
- Consumes: ADR-006 archived wasm boundary, ADR-011 host-cli fixed `deviceId` exception, existing FFI verify script.
- Produces: documented decision on FFI verify gate alignment, proof that hardcoded dev `deviceId` did not leak to delivery paths, and no `Harness::new` dead-code warning.

- [ ] **Step 1: Audit fixed `deviceId` exception**

Run:

```bash
rg -n 'deviceId|device_id|dev-bin|host-cli|ADR-011|fixed' crates docs scripts
```

Expected: any fixed development `deviceId` remains scoped to `crates/helix-host-cli/src/main.rs` and documented in `docs/harness/decisions.md`; native/FFI/Tauri delivery paths do not introduce the same hardcoded credential.

- [ ] **Step 2: Decide FFI verify gate alignment**

Inspect `scripts/ffi-verify.sh` and `scripts/gate.sh`. Either wire `scripts/ffi-verify.sh --no-demo` into the default gate, or document why it remains a separate CI/manual trigger. The decision must be explicit.

- [ ] **Step 3: Clear `helix-im` test warning**

If `Harness::new` remains intentionally available for integration tests, add a narrow `#[allow(dead_code)]` on that helper. If it is unused and not useful, remove it. Verify with:

```bash
cargo test -p helix-im --no-run
```

Expected: no `Harness::new` warning.

- [ ] **Step 4: Verify Task 4**

Run:

```bash
cargo test -p helix-im --no-run
scripts/ffi-verify.sh --no-demo
scripts/gate.sh
```

Expected:
- no helix-im dead-code warning;
- FFI verify result is captured;
- default gate result is captured.

- [ ] **Step 5: Request task review**

Generate a Task 4 review package and dispatch a task reviewer. Critical and Important findings must be fixed and re-reviewed.

### Task 5: Final Whole-Branch Review and Closure

**Files:**
- Inspect: all files changed by Tasks 1-4.
- Modify only if final review finds Critical or Important issues.

**Interfaces:**
- Consumes: reviewed task changes and final verification evidence.
- Produces: final reviewed branch state ready for user decision on commit/PR/merge.

- [ ] **Step 1: Run final verification**

Run:

```bash
git diff --check fa07ff1..HEAD
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-im
cargo test -p helix-core -p helix-driver-host -p helix-driver-native -p helix-driver-ffi
scripts/gate.sh
scripts/ffi-verify.sh --no-demo
```

Expected: all commands pass, or any environmental failure is isolated with exact output and a follow-up decision.

- [ ] **Step 2: Run GitNexus detect changes**

Run:

```text
detect_changes({scope: "compare", base_ref: "main"})
```

Expected: changed symbols and flows match the planned review-fix scope.

- [ ] **Step 3: Dispatch final code reviewer**

Use `superpowers:requesting-code-review` with the full branch diff from the branch base to HEAD. Critical and Important findings must be fixed before closure.

- [ ] **Step 4: Finish branch**

Use `superpowers:finishing-a-development-branch` to present final options: leave changes unstaged, stage/commit, push/PR, or merge if explicitly requested.
