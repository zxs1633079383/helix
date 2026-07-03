# Helix Scripts Entropy Reduction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce `scripts/` to a small set of maintained, discoverable entrypoints without breaking Helix gates or cses-im-server联调.

**Architecture:** Keep canonical scripts by workflow: branch/review/structure gates, FFI validation, coverage, and four host-cli联调 entrypoints (`dev-loop`, `host-cli-session`, `host-cli-FULL-uc-e2e`, `host-cli-dual-account-e2e`). Remove one-off host-cli UC scripts that are unreferenced or superseded by the canonical entrypoints, then add a `scripts/README.md` map.

**Tech Stack:** Bash scripts, `rg` reference audit, `bash -n` syntax verification.

## Global Constraints

- Do not change Rust source code.
- Do not touch existing untracked `crates/helix-driver-ffi/docs/*` files.
- Preserve scripts called by `scripts/gate.sh`, `docs/harness/OPERATING.md`, or project hooks.
- Preserve `scripts/lib-config.sh` as the shared `helix.config.yaml` reader for cses-im-server `:8066`.

---

### Task 1: Delete Superseded UC-Specific Host-CLI Scripts

**Files:**
- Delete: `scripts/host-cli-channel-admin-e2e.sh`
- Delete: `scripts/host-cli-go-chain-e2e-persistent.sh`
- Delete: `scripts/host-cli-go-chain-e2e.sh`
- Delete: `scripts/host-cli-go-e2e.sh`
- Delete: `scripts/host-cli-sync-gap-e2e.sh`
- Delete: `scripts/host-cli-uc15-revoke-e2e.sh`
- Delete: `scripts/host-cli-uc16-edit-e2e.sh`
- Delete: `scripts/host-cli-uc24-replies-e2e.sh`
- Delete: `scripts/uc22-load-older-e2e.sh`

**Interfaces:**
- Consumes: existing `host-cli` command semantics.
- Produces: reduced script surface where `host-cli-FULL-uc-e2e.sh`, `host-cli-session.sh`, `host-cli-dual-account-e2e.sh`, and `dev-loop.sh` are the supported联调 entrypoints.

- [ ] **Step 1: Verify candidate references**

Run:
```bash
for s in host-cli-channel-admin-e2e.sh host-cli-go-chain-e2e-persistent.sh host-cli-go-chain-e2e.sh host-cli-go-e2e.sh host-cli-sync-gap-e2e.sh host-cli-uc15-revoke-e2e.sh host-cli-uc16-edit-e2e.sh host-cli-uc24-replies-e2e.sh uc22-load-older-e2e.sh; do
  rg -n "$s" . --glob '!target/**' --glob '!worktrees/**'
done
```
Expected: only historical docs/comments, no gate/hook hard dependency.

- [ ] **Step 2: Delete candidates**

Remove the listed one-off scripts.

- [ ] **Step 3: Replace remaining references**

Change docs that point users to deleted scripts so they point to `scripts/host-cli-session.sh`, `scripts/host-cli-FULL-uc-e2e.sh`, or `scripts/dev-loop.sh`.

### Task 2: Add Maintained Scripts Map

**Files:**
- Create: `scripts/README.md`

**Interfaces:**
- Consumes: retained script names.
- Produces: a single map for “which script should I run?”

- [ ] **Step 1: Document retained scripts**

Create `scripts/README.md` with categories:
- Commit/review gates.
- cses-im-server联调.
- FFI/coverage validation.
- Branch/worktree helpers.

- [ ] **Step 2: Document removed-script rule**

State that UC-specific one-off scripts should be modeled as commands fed into `host-cli-session.sh` unless they become a recurring gate.

### Task 3: Verify No Broken References

**Files:**
- Modify: `SESSION.md`

**Interfaces:**
- Consumes: final script set.
- Produces: verified cleanup record.

- [ ] **Step 1: Run syntax checks**

Run:
```bash
bash -n scripts/*.sh
```
Expected: exit 0.

- [ ] **Step 2: Run reference check**

Run:
```bash
for s in host-cli-channel-admin-e2e.sh host-cli-go-chain-e2e-persistent.sh host-cli-go-chain-e2e.sh host-cli-go-e2e.sh host-cli-sync-gap-e2e.sh host-cli-uc15-revoke-e2e.sh host-cli-uc16-edit-e2e.sh host-cli-uc24-replies-e2e.sh uc22-load-older-e2e.sh; do
  ! rg -n "$s" docs .claude crates scripts README.md AGENTS.md CLAUDE.md SESSION.md --glob '!target/**' --glob '!worktrees/**'
done
```
Expected: no deleted script references remain.

- [ ] **Step 3: Record DONE**

Append a `✅ DONE` line to `SESSION.md` with script count, retained canonical entrypoints, and verification commands.
