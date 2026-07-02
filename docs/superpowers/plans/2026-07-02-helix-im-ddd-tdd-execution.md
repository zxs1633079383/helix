# helix-im DDD TDD Execution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Execute GitLab issues #9-#20 for `helix-im` DDD package layering, starting at #9, with structural RED-GREEN TDD before every issue.

**Architecture:** This is a behavior-preserving directory refactor. A reusable structural gate script provides per-issue RED/GREEN checks for target paths, old-path cleanup, thin `mod.rs` re-exports, whitelist/docs mirror updates, crate instruction docs, and coverage review evidence.

**Tech Stack:** Rust workspace, Bash gate scripts, Cargo tests, GitLab issues via `glab`, existing Helix `structure-gate.sh` and commit convention.

## Global Constraints

- Pure directory/module refactor: no business logic, function signature, branch-order, schema, wire, or protocol changes.
- Every issue #9-#20 must start with structural RED evidence before production/refactor changes.
- The RED failure must be caused by target structure not yet existing, not syntax errors or broken test code.
- Every issue must end with GREEN evidence from the same structural check plus the issue's required Cargo/gate commands.
- `mod.rs` files created by this plan must be thin re-export shells only.
- Do not modify `.agents/`, `.codex/`, root `AGENTS.md`, root `CLAUDE.md`, or unrelated untracked work.
- `crates/helix-im/CLAUDE.md` and `crates/helix-im/AGENTS.md` must remain byte-identical after #19.
- Structure whitelist baseline numbers may not increase.
- Follow Cargo discipline: do not run cargo after every tiny move; run once per issue/phase as specified and batch-fix compile errors.

---

## File Structure

- Create `scripts/ddd-structure-gate.sh`: per-issue structural TDD gate. It must accept one or more issue numbers (`9`, `10`, ..., `20`, or `all`) and return non-zero until that issue's target structure is present.
- Modify `crates/helix-im/src/lib.rs`: module declarations and compatibility aliases as each domain moves.
- Move files under `crates/helix-im/src/**` according to GitLab issues #9-#18.
- Modify `scripts/structure-gate.sh`: update whitelist paths for #9 and #18 only, baseline values unchanged.
- Modify `.claude/rules/code-structure-ddd.md`: mirror whitelist path changes for #9 and #18.
- Modify `crates/helix-im/CLAUDE.md` and `crates/helix-im/AGENTS.md`: final module map for #19.
- Create `docs/review/coverage-ddd-match-key-review.md`: human-readable #20 conclusion for `scripts/coverage.sh` `MATCH_KEY`.
- Use each commit body to record RED/GREEN commands for that issue.

## Task 1: Create Structural TDD Gate Script

**Files:**
- Create: `scripts/ddd-structure-gate.sh`

**Interfaces:**
- Consumes: filesystem paths in `crates/helix-im/src/**`, `scripts/structure-gate.sh`, `.claude/rules/code-structure-ddd.md`, `crates/helix-im/CLAUDE.md`, `crates/helix-im/AGENTS.md`, `docs/review/coverage-ddd-match-key-review.md`
- Produces: command `scripts/ddd-structure-gate.sh <issue|all>` with exit `0` for GREEN and `1` for RED

- [ ] **Step 1: Write the failing structural gate**

Create `scripts/ddd-structure-gate.sh` with this content:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

fail=0

mark_fail() {
  echo "DDD-STRUCT   x $*"
  fail=1
}

mark_ok() {
  echo "DDD-STRUCT   · $*"
}

require_file() {
  local path="$1"
  [ -f "$path" ] && mark_ok "exists $path" || mark_fail "missing $path"
}

forbid_file() {
  local path="$1"
  [ ! -e "$path" ] && mark_ok "removed $path" || mark_fail "old path still exists $path"
}

require_dir() {
  local path="$1"
  [ -d "$path" ] && mark_ok "exists $path/" || mark_fail "missing dir $path/"
}

require_grep() {
  local pattern="$1"
  local path="$2"
  grep -F "$pattern" "$path" >/dev/null 2>&1 && mark_ok "grep '$pattern' in $path" || mark_fail "missing '$pattern' in $path"
}

forbid_grep() {
  local pattern="$1"
  local path="$2"
  grep -F "$pattern" "$path" >/dev/null 2>&1 && mark_fail "forbidden '$pattern' still in $path" || mark_ok "absent '$pattern' in $path"
}

require_thin_mod() {
  local path="$1"
  require_file "$path"
  [ -f "$path" ] || return 0
  if grep -nE 'fn |impl |struct |enum |match |for |while |if |let ' "$path"; then
    mark_fail "$path is not a thin re-export mod"
  else
    mark_ok "$path is thin"
  fi
}

check_9() {
  echo "DDD-STRUCT #9 channel"
  require_file crates/helix-im/src/channel/gate.rs
  require_file crates/helix-im/src/channel/gate_tests.rs
  require_file crates/helix-im/src/channel/lifecycle.rs
  require_file crates/helix-im/src/channel/lifecycle_scan.rs
  require_file crates/helix-im/src/channel/older_context.rs
  require_file crates/helix-im/src/channel/older_context_tests.rs
  require_file crates/helix-im/src/channel/write/mod.rs
  require_file crates/helix-im/src/channel/write/members.rs
  require_file crates/helix-im/src/channel/write/patch.rs
  require_file crates/helix-im/src/channel/write/post.rs
  require_file crates/helix-im/src/channel/write/program.rs
  require_thin_mod crates/helix-im/src/channel/mod.rs
  forbid_file crates/helix-im/src/channel.rs
  forbid_file crates/helix-im/src/channel_tests.rs
  forbid_file crates/helix-im/src/channel_write
  require_grep "crates/helix-im/src/channel/gate.rs 579" scripts/structure-gate.sh
  forbid_grep "crates/helix-im/src/channel.rs 579" scripts/structure-gate.sh
  require_grep "helix-im/src/channel/gate.rs" .claude/rules/code-structure-ddd.md
}

check_10() {
  echo "DDD-STRUCT #10 sync"
  require_file crates/helix-im/src/sync/flow.rs
  require_file crates/helix-im/src/sync/increment.rs
  require_file crates/helix-im/src/sync/scheduler.rs
  require_file crates/helix-im/src/sync/scheduler_tests.rs
  require_file crates/helix-im/src/sync/session.rs
  require_thin_mod crates/helix-im/src/sync/mod.rs
  forbid_file crates/helix-im/src/sync_flow.rs
  forbid_file crates/helix-im/src/sync_increment.rs
  forbid_file crates/helix-im/src/sync_scheduler.rs
  forbid_file crates/helix-im/src/sync_scheduler_tests.rs
  forbid_file crates/helix-im/src/sync_session.rs
}

check_11() {
  echo "DDD-STRUCT #11 send"
  require_file crates/helix-im/src/send/flow.rs
  require_file crates/helix-im/src/send/reconcile.rs
  require_file crates/helix-im/src/send/pending.rs
  require_file crates/helix-im/src/send/pong_compensate.rs
  require_file crates/helix-im/src/send/pong_compensate_tests.rs
  require_thin_mod crates/helix-im/src/send/mod.rs
  require_file crates/helix-im/src/correlation.rs
  forbid_file crates/helix-im/src/send_flow.rs
  forbid_file crates/helix-im/src/send_reconcile.rs
  forbid_file crates/helix-im/src/pending_send.rs
  forbid_file crates/helix-im/src/pong_compensate.rs
  forbid_file crates/helix-im/src/pong_compensate_tests.rs
}

check_12() {
  echo "DDD-STRUCT #12 query"
  require_file crates/helix-im/src/query/core.rs
  require_file crates/helix-im/src/query/core_tests.rs
  require_file crates/helix-im/src/query/dispatch.rs
  require_file crates/helix-im/src/query/dialog_render.rs
  require_file crates/helix-im/src/query/read_relay.rs
  require_file crates/helix-im/src/query/render_ready/core.rs
  require_file crates/helix-im/src/query/render_ready/core_tests.rs
  require_file crates/helix-im/src/query/render_ready/members.rs
  require_file crates/helix-im/src/query/render_ready/members_tests.rs
  require_file crates/helix-im/src/query/render_ready/replies.rs
  require_file crates/helix-im/src/query/render_ready/replies_tests.rs
  require_thin_mod crates/helix-im/src/query/mod.rs
  require_thin_mod crates/helix-im/src/query/render_ready/mod.rs
  forbid_file crates/helix-im/src/query.rs
  forbid_file crates/helix-im/src/query_tests.rs
  forbid_file crates/helix-im/src/query_dispatch.rs
  forbid_file crates/helix-im/src/query_dialog_render.rs
  forbid_file crates/helix-im/src/read_relay.rs
  forbid_file crates/helix-im/src/render_ready.rs
  forbid_file crates/helix-im/src/render_ready_tests.rs
  forbid_file crates/helix-im/src/render_ready_members.rs
  forbid_file crates/helix-im/src/render_ready_members_tests.rs
  forbid_file crates/helix-im/src/render_ready_replies.rs
  forbid_file crates/helix-im/src/render_ready_replies_tests.rs
}

check_13() {
  echo "DDD-STRUCT #13 todo"
  require_file crates/helix-im/src/todo/core.rs
  require_file crates/helix-im/src/todo/core_tests.rs
  require_thin_mod crates/helix-im/src/todo/mod.rs
  forbid_file crates/helix-im/src/todo.rs
  forbid_file crates/helix-im/src/todo_tests.rs
}

check_14() {
  echo "DDD-STRUCT #14 outbound/channel_admin"
  require_file crates/helix-im/src/outbound/channel_admin/change.rs
  require_file crates/helix-im/src/outbound/channel_admin/change_dedicated.rs
  require_file crates/helix-im/src/outbound/channel_admin/change_dedicated_tests.rs
  require_file crates/helix-im/src/outbound/channel_admin/create.rs
  require_file crates/helix-im/src/outbound/channel_admin/existing.rs
  require_file crates/helix-im/src/outbound/channel_admin/member_admin.rs
  require_file crates/helix-im/src/outbound/channel_admin/pinned.rs
  require_file crates/helix-im/src/outbound/channel_admin/read.rs
  require_file crates/helix-im/src/outbound/channel_admin/read_tests.rs
  require_thin_mod crates/helix-im/src/outbound/channel_admin/mod.rs
  forbid_file crates/helix-im/src/outbound/channel_change.rs
  forbid_file crates/helix-im/src/outbound/channel_change_dedicated.rs
  forbid_file crates/helix-im/src/outbound/channel_change_dedicated_tests.rs
  forbid_file crates/helix-im/src/outbound/channel_create.rs
  forbid_file crates/helix-im/src/outbound/channel_existing.rs
  forbid_file crates/helix-im/src/outbound/channel_member_admin.rs
  forbid_file crates/helix-im/src/outbound/channel_pinned.rs
  forbid_file crates/helix-im/src/outbound/channel_read.rs
  forbid_file crates/helix-im/src/outbound/channel_read_tests.rs
}

check_15() {
  echo "DDD-STRUCT #15 outbound/posts"
  require_file crates/helix-im/src/outbound/posts/props.rs
  require_file crates/helix-im/src/outbound/posts/announcement.rs
  require_file crates/helix-im/src/outbound/posts/approval.rs
  require_file crates/helix-im/src/outbound/posts/existing.rs
  require_file crates/helix-im/src/outbound/posts/read.rs
  require_file crates/helix-im/src/outbound/posts/read_ext.rs
  require_file crates/helix-im/src/outbound/posts/relay.rs
  require_thin_mod crates/helix-im/src/outbound/posts/mod.rs
  forbid_file crates/helix-im/src/outbound/post_props.rs
  forbid_file crates/helix-im/src/outbound/posts_announcement.rs
  forbid_file crates/helix-im/src/outbound/posts_approval.rs
  forbid_file crates/helix-im/src/outbound/posts_existing.rs
  forbid_file crates/helix-im/src/outbound/posts_read.rs
  forbid_file crates/helix-im/src/outbound/posts_read_ext.rs
  forbid_file crates/helix-im/src/outbound/posts_relay.rs
}

check_16() {
  echo "DDD-STRUCT #16 outbound/engagement"
  require_file crates/helix-im/src/outbound/engagement/vote_score.rs
  require_file crates/helix-im/src/outbound/engagement/vote_score_tests.rs
  require_file crates/helix-im/src/outbound/engagement/quick_reply.rs
  require_file crates/helix-im/src/outbound/engagement/urgent.rs
  require_file crates/helix-im/src/outbound/engagement/template_received.rs
  require_thin_mod crates/helix-im/src/outbound/engagement/mod.rs
  forbid_file crates/helix-im/src/outbound/vote_score.rs
  forbid_file crates/helix-im/src/outbound/vote_score_tests.rs
  forbid_file crates/helix-im/src/outbound/quick_reply.rs
  forbid_file crates/helix-im/src/outbound/urgent.rs
  forbid_file crates/helix-im/src/outbound/template_received.rs
}

check_17() {
  echo "DDD-STRUCT #17 outbound/user_misc"
  require_file crates/helix-im/src/outbound/user_misc/core.rs
  require_file crates/helix-im/src/outbound/user_misc/helpers.rs
  require_file crates/helix-im/src/outbound/user_misc/core_tests.rs
  require_thin_mod crates/helix-im/src/outbound/user_misc/mod.rs
  forbid_file crates/helix-im/src/outbound/user_misc.rs
  forbid_file crates/helix-im/src/outbound/user_misc_helpers.rs
  forbid_file crates/helix-im/src/outbound/user_misc_tests.rs
}

check_18() {
  echo "DDD-STRUCT #18 ws/parser"
  require_file crates/helix-im/src/ws/parser/core.rs
  require_file crates/helix-im/src/ws/parser/post_props.rs
  require_thin_mod crates/helix-im/src/ws/parser/mod.rs
  forbid_file crates/helix-im/src/parser.rs
  forbid_file crates/helix-im/src/parser_post_props.rs
  require_grep "pub mod parser;" crates/helix-im/src/ws/mod.rs
  require_grep "crates/helix-im/src/ws/parser/core.rs 430" scripts/structure-gate.sh
  forbid_grep "crates/helix-im/src/parser.rs 430" scripts/structure-gate.sh
  require_grep "helix-im/src/ws/parser/core.rs" .claude/rules/code-structure-ddd.md
}

check_19() {
  echo "DDD-STRUCT #19 helix-im instructions"
  diff -u crates/helix-im/CLAUDE.md crates/helix-im/AGENTS.md >/dev/null 2>&1 || mark_fail "CLAUDE.md and AGENTS.md differ"
  require_grep "channel/" crates/helix-im/CLAUDE.md
  require_grep "send/" crates/helix-im/CLAUDE.md
  require_grep "sync/" crates/helix-im/CLAUDE.md
  require_grep "query/" crates/helix-im/CLAUDE.md
  require_grep "todo/" crates/helix-im/CLAUDE.md
  require_grep "outbound/" crates/helix-im/CLAUDE.md
  require_grep "ws/" crates/helix-im/CLAUDE.md
  require_grep "acl/" crates/helix-im/CLAUDE.md
  require_grep "职责" crates/helix-im/CLAUDE.md
  require_grep "怎么用" crates/helix-im/CLAUDE.md
  require_grep "依赖" crates/helix-im/CLAUDE.md
}

check_20() {
  echo "DDD-STRUCT #20 coverage MATCH_KEY review"
  require_file docs/review/coverage-ddd-match-key-review.md
  require_grep 'MATCH_KEY="${FILE#crates/helix-im/src/}"' docs/review/coverage-ddd-match-key-review.md
  require_grep "目录变深" docs/review/coverage-ddd-match-key-review.md
  require_grep "结论" docs/review/coverage-ddd-match-key-review.md
}

run_one() {
  case "$1" in
    9) check_9 ;;
    10) check_10 ;;
    11) check_11 ;;
    12) check_12 ;;
    13) check_13 ;;
    14) check_14 ;;
    15) check_15 ;;
    16) check_16 ;;
    17) check_17 ;;
    18) check_18 ;;
    19) check_19 ;;
    20) check_20 ;;
    all) for n in 9 10 11 12 13 14 15 16 17 18 19 20; do run_one "$n"; done ;;
    *) echo "usage: $0 <9|10|...|20|all> [more issues]" >&2; exit 2 ;;
  esac
}

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <9|10|...|20|all> [more issues]" >&2
  exit 2
fi

for issue in "$@"; do
  run_one "$issue"
done

if [ "$fail" -eq 0 ]; then
  echo "DDD-STRUCT PASS"
else
  echo "DDD-STRUCT FAIL"
fi
exit "$fail"
```

- [ ] **Step 2: Run #9 RED**

Run:

```bash
chmod +x scripts/ddd-structure-gate.sh
scripts/ddd-structure-gate.sh 9
```

Expected: FAIL with missing `crates/helix-im/src/channel/gate.rs`, missing `channel/write/`, and old-path findings.

- [ ] **Step 3: Commit the reusable RED gate**

```bash
git add scripts/ddd-structure-gate.sh
git commit -m "test(im): 新增 DDD 分包结构红测闸门"
```

Commit body must include that #9 RED was observed and expected.

## Task 2: Issue #9 channel/ Domain Migration

**Files:**
- Move: `crates/helix-im/src/channel.rs` → `crates/helix-im/src/channel/gate.rs`
- Move: `crates/helix-im/src/channel_tests.rs` → `crates/helix-im/src/channel/gate_tests.rs`
- Move: `crates/helix-im/src/lifecycle.rs` → `crates/helix-im/src/channel/lifecycle.rs`
- Move: `crates/helix-im/src/lifecycle_scan.rs` → `crates/helix-im/src/channel/lifecycle_scan.rs`
- Move: `crates/helix-im/src/older_context.rs` → `crates/helix-im/src/channel/older_context.rs`
- Move: `crates/helix-im/src/older_context_tests.rs` → `crates/helix-im/src/channel/older_context_tests.rs`
- Move: `crates/helix-im/src/channel_write/` → `crates/helix-im/src/channel/write/`
- Create: `crates/helix-im/src/channel/mod.rs`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `scripts/structure-gate.sh`
- Modify: `.claude/rules/code-structure-ddd.md`

**Interfaces:**
- Consumes: existing `crate::channel::*`, `crate::channel_write::*`, lifecycle, and older context imports
- Produces: `crate::channel::gate`, `crate::channel::write`, and compatibility aliases from `lib.rs`

- [ ] **Step 1: Verify #9 RED is fresh**

Run:

```bash
scripts/ddd-structure-gate.sh 9
```

Expected: FAIL for missing target channel paths.

- [ ] **Step 2: Move files**

Run:

```bash
mkdir -p crates/helix-im/src/channel
git mv crates/helix-im/src/channel.rs crates/helix-im/src/channel/gate.rs
git mv crates/helix-im/src/channel_tests.rs crates/helix-im/src/channel/gate_tests.rs
git mv crates/helix-im/src/lifecycle.rs crates/helix-im/src/channel/lifecycle.rs
git mv crates/helix-im/src/lifecycle_scan.rs crates/helix-im/src/channel/lifecycle_scan.rs
git mv crates/helix-im/src/older_context.rs crates/helix-im/src/channel/older_context.rs
git mv crates/helix-im/src/older_context_tests.rs crates/helix-im/src/channel/older_context_tests.rs
git mv crates/helix-im/src/channel_write crates/helix-im/src/channel/write
```

- [ ] **Step 3: Create `channel/mod.rs`**

```rust
//! Channel aggregate root and channel write model.

pub mod gate;
mod lifecycle;
mod lifecycle_scan;
pub mod older_context;
pub mod write;

pub use gate::*;
pub(crate) use lifecycle::*;
pub(crate) use lifecycle_scan::*;
```

- [ ] **Step 4: Update `lib.rs` channel declarations**

Replace the old declarations:

```rust
pub mod channel;
pub mod channel_write;
mod lifecycle;
mod lifecycle_scan;
pub mod older_context;
```

with:

```rust
pub mod channel;
pub use channel::write as channel_write;
pub use channel::older_context;
```

Keep all unrelated declarations unchanged.

- [ ] **Step 5: Update whitelist mirrors**

In `scripts/structure-gate.sh`, replace:

```bash
"crates/helix-im/src/channel.rs 579"
```

with:

```bash
"crates/helix-im/src/channel/gate.rs 579"
```

In `.claude/rules/code-structure-ddd.md`, replace the table path `helix-im/src/channel.rs` with `helix-im/src/channel/gate.rs`. Do not change the baseline number.

- [ ] **Step 6: Run #9 GREEN checks**

Run:

```bash
scripts/ddd-structure-gate.sh 9
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-im
./scripts/structure-gate.sh
```

Expected: all PASS. If `cargo check` reports missing imports, update only module paths/visibility needed for the move; do not alter logic.

- [ ] **Step 7: Commit #9**

```bash
git add crates/helix-im/src crates/helix-im/src/channel scripts/structure-gate.sh .claude/rules/code-structure-ddd.md
git commit -m "refactor(im): 迁移 channel 域到限界上下文目录"
```

Commit body must include RED command, GREEN commands, and issue `#9`.

## Task 3: Issue #10 sync/ Domain Migration

**Files:**
- Move: `sync_flow.rs` → `sync/flow.rs`
- Move: `sync_increment.rs` → `sync/increment.rs`
- Move: `sync_scheduler.rs` → `sync/scheduler.rs`
- Move: `sync_scheduler_tests.rs` → `sync/scheduler_tests.rs`
- Move: `sync_session.rs` → `sync/session.rs`
- Create: `sync/mod.rs`
- Modify: `lib.rs`

**Interfaces:**
- Produces: `crate::sync::{flow, increment, scheduler, session}` and compatibility aliases for old paths

- [ ] **Step 1: Run #10 RED**

```bash
scripts/ddd-structure-gate.sh 10
```

Expected: FAIL for missing `crates/helix-im/src/sync/*.rs`.

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/sync
git mv crates/helix-im/src/sync_flow.rs crates/helix-im/src/sync/flow.rs
git mv crates/helix-im/src/sync_increment.rs crates/helix-im/src/sync/increment.rs
git mv crates/helix-im/src/sync_scheduler.rs crates/helix-im/src/sync/scheduler.rs
git mv crates/helix-im/src/sync_scheduler_tests.rs crates/helix-im/src/sync/scheduler_tests.rs
git mv crates/helix-im/src/sync_session.rs crates/helix-im/src/sync/session.rs
```

- [ ] **Step 3: Create `sync/mod.rs`**

```rust
//! Incremental sync cursor engine and sync session state.

pub(crate) mod flow;
pub mod increment;
pub mod scheduler;
pub mod session;

pub(crate) use flow::*;
```

- [ ] **Step 4: Update `lib.rs`**

Replace old sync declarations with:

```rust
pub mod sync;
pub(crate) use sync::flow as sync_flow;
pub use sync::increment as sync_increment;
pub use sync::scheduler as sync_scheduler;
pub use sync::session as sync_session;
```

- [ ] **Step 5: Run #10 GREEN**

```bash
scripts/ddd-structure-gate.sh 10
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 6: Commit #10**

Commit message: `refactor(im): 迁移 sync 域到限界上下文目录`

## Task 4: Issue #11 send/ Domain Migration

**Files:**
- Move: `send_flow.rs` → `send/flow.rs`
- Move: `send_reconcile.rs` → `send/reconcile.rs`
- Move: `pending_send.rs` → `send/pending.rs`
- Move: `pong_compensate.rs` → `send/pong_compensate.rs`
- Move: `pong_compensate_tests.rs` → `send/pong_compensate_tests.rs`
- Create: `send/mod.rs`
- Modify: `lib.rs`

**Interfaces:**
- Produces: `crate::send::{flow, reconcile, pending, pong_compensate}` and compatibility aliases

- [ ] **Step 1: Run #11 RED**

```bash
scripts/ddd-structure-gate.sh 11
```

Expected: FAIL for missing `send/` paths.

- [ ] **Step 2: Move files and create mod**

```bash
mkdir -p crates/helix-im/src/send
git mv crates/helix-im/src/send_flow.rs crates/helix-im/src/send/flow.rs
git mv crates/helix-im/src/send_reconcile.rs crates/helix-im/src/send/reconcile.rs
git mv crates/helix-im/src/pending_send.rs crates/helix-im/src/send/pending.rs
git mv crates/helix-im/src/pong_compensate.rs crates/helix-im/src/send/pong_compensate.rs
git mv crates/helix-im/src/pong_compensate_tests.rs crates/helix-im/src/send/pong_compensate_tests.rs
```

`send/mod.rs`:

```rust
//! Optimistic send flow, pending send state, and echo reconciliation.

pub(crate) mod flow;
pub mod pending;
pub mod pong_compensate;
pub mod reconcile;

pub(crate) use flow::*;
```

- [ ] **Step 3: Update `lib.rs` aliases**

```rust
pub mod send;
pub(crate) use send::flow as send_flow;
pub use send::pending as pending_send;
pub use send::pong_compensate;
pub use send::reconcile as send_reconcile;
```

- [ ] **Step 4: Run #11 GREEN**

```bash
scripts/ddd-structure-gate.sh 11
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #11**

Commit message: `refactor(im): 迁移 send 域到限界上下文目录`

## Task 5: Issue #12 query/ Domain Migration

**Files:** move the `query*`, `read_relay`, and `render_ready*` files to `query/` and `query/render_ready/`; modify `lib.rs`.

**Interfaces:** produces `crate::query::{core, dispatch, dialog_render, read_relay, render_ready}` plus compatibility aliases.

- [ ] **Step 1: Run #12 RED**

```bash
scripts/ddd-structure-gate.sh 12
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/query/render_ready
git mv crates/helix-im/src/query.rs crates/helix-im/src/query/core.rs
git mv crates/helix-im/src/query_tests.rs crates/helix-im/src/query/core_tests.rs
git mv crates/helix-im/src/query_dispatch.rs crates/helix-im/src/query/dispatch.rs
git mv crates/helix-im/src/query_dialog_render.rs crates/helix-im/src/query/dialog_render.rs
git mv crates/helix-im/src/read_relay.rs crates/helix-im/src/query/read_relay.rs
git mv crates/helix-im/src/render_ready.rs crates/helix-im/src/query/render_ready/core.rs
git mv crates/helix-im/src/render_ready_tests.rs crates/helix-im/src/query/render_ready/core_tests.rs
git mv crates/helix-im/src/render_ready_members.rs crates/helix-im/src/query/render_ready/members.rs
git mv crates/helix-im/src/render_ready_members_tests.rs crates/helix-im/src/query/render_ready/members_tests.rs
git mv crates/helix-im/src/render_ready_replies.rs crates/helix-im/src/query/render_ready/replies.rs
git mv crates/helix-im/src/render_ready_replies_tests.rs crates/helix-im/src/query/render_ready/replies_tests.rs
```

- [ ] **Step 3: Create mod files**

`query/mod.rs`:

```rust
//! Query-side projections and render-ready read models.

pub mod core;
pub(crate) mod dialog_render;
pub(crate) mod dispatch;
pub mod read_relay;
pub mod render_ready;

pub use core::*;
```

`query/render_ready/mod.rs`:

```rust
//! Render-ready projection builders.

pub mod core;
pub mod members;
pub mod replies;

pub use core::*;
pub use members::*;
pub use replies::*;
```

- [ ] **Step 4: Update `lib.rs` aliases**

```rust
pub mod query;
pub(crate) use query::dialog_render as query_dialog_render;
pub(crate) use query::dispatch as query_dispatch;
pub use query::read_relay;
pub use query::render_ready::core as render_ready;
pub use query::render_ready::members as render_ready_members;
pub use query::render_ready::replies as render_ready_replies;
```

- [ ] **Step 5: Run #12 GREEN**

```bash
scripts/ddd-structure-gate.sh 12
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 6: Commit #12**

Commit message: `refactor(im): 迁移 query 读侧域到限界上下文目录`

## Task 6: Issue #13 todo/ Domain Migration

**Files:** `todo.rs`, `todo_tests.rs`, `todo/mod.rs`, `lib.rs`.

- [ ] **Step 1: Run #13 RED**

```bash
scripts/ddd-structure-gate.sh 13
```

- [ ] **Step 2: Move files and create mod**

```bash
mkdir -p crates/helix-im/src/todo
git mv crates/helix-im/src/todo.rs crates/helix-im/src/todo/core.rs
git mv crates/helix-im/src/todo_tests.rs crates/helix-im/src/todo/core_tests.rs
```

`todo/mod.rs`:

```rust
//! Todo domain assembly.

pub mod core;

pub use core::*;
```

- [ ] **Step 3: Update `lib.rs`**

Keep `pub mod todo;`; no compatibility alias should be needed if `todo/mod.rs` re-exports `core`.

- [ ] **Step 4: Run #13 GREEN**

```bash
scripts/ddd-structure-gate.sh 13
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #13**

Commit message: `refactor(im): 迁移 todo 域到限界上下文目录`

## Task 7: Issue #14 outbound/channel_admin Split

**Files:** outbound channel admin files, `outbound/channel_admin/mod.rs`, `outbound/mod.rs`.

- [ ] **Step 1: Run #14 RED**

```bash
scripts/ddd-structure-gate.sh 14
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/outbound/channel_admin
git mv crates/helix-im/src/outbound/channel_change.rs crates/helix-im/src/outbound/channel_admin/change.rs
git mv crates/helix-im/src/outbound/channel_change_dedicated.rs crates/helix-im/src/outbound/channel_admin/change_dedicated.rs
git mv crates/helix-im/src/outbound/channel_change_dedicated_tests.rs crates/helix-im/src/outbound/channel_admin/change_dedicated_tests.rs
git mv crates/helix-im/src/outbound/channel_create.rs crates/helix-im/src/outbound/channel_admin/create.rs
git mv crates/helix-im/src/outbound/channel_existing.rs crates/helix-im/src/outbound/channel_admin/existing.rs
git mv crates/helix-im/src/outbound/channel_member_admin.rs crates/helix-im/src/outbound/channel_admin/member_admin.rs
git mv crates/helix-im/src/outbound/channel_pinned.rs crates/helix-im/src/outbound/channel_admin/pinned.rs
git mv crates/helix-im/src/outbound/channel_read.rs crates/helix-im/src/outbound/channel_admin/read.rs
git mv crates/helix-im/src/outbound/channel_read_tests.rs crates/helix-im/src/outbound/channel_admin/read_tests.rs
```

- [ ] **Step 3: Create `outbound/channel_admin/mod.rs`**

```rust
//! Outbound channel administration command builders.

pub mod change;
pub mod change_dedicated;
pub mod create;
pub mod existing;
pub mod member_admin;
pub mod pinned;
pub mod read;
```

- [ ] **Step 4: Update `outbound/mod.rs`**

Add `pub mod channel_admin;` and these compatibility aliases:

```rust
pub use channel_admin::change as channel_change;
pub use channel_admin::change_dedicated as channel_change_dedicated;
pub use channel_admin::create as channel_create;
pub use channel_admin::existing as channel_existing;
pub use channel_admin::member_admin as channel_member_admin;
pub use channel_admin::pinned as channel_pinned;
pub use channel_admin::read as channel_read;
```

- [ ] **Step 5: Run #14 GREEN**

```bash
scripts/ddd-structure-gate.sh 14
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 6: Commit #14**

Commit message: `refactor(im): 拆分 outbound channel_admin 子域`

## Task 8: Issue #15 outbound/posts Split

**Files:** outbound post files, `outbound/posts/mod.rs`, `outbound/mod.rs`.

- [ ] **Step 1: Run #15 RED**

```bash
scripts/ddd-structure-gate.sh 15
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/outbound/posts
git mv crates/helix-im/src/outbound/post_props.rs crates/helix-im/src/outbound/posts/props.rs
git mv crates/helix-im/src/outbound/posts_announcement.rs crates/helix-im/src/outbound/posts/announcement.rs
git mv crates/helix-im/src/outbound/posts_approval.rs crates/helix-im/src/outbound/posts/approval.rs
git mv crates/helix-im/src/outbound/posts_existing.rs crates/helix-im/src/outbound/posts/existing.rs
git mv crates/helix-im/src/outbound/posts_read.rs crates/helix-im/src/outbound/posts/read.rs
git mv crates/helix-im/src/outbound/posts_read_ext.rs crates/helix-im/src/outbound/posts/read_ext.rs
git mv crates/helix-im/src/outbound/posts_relay.rs crates/helix-im/src/outbound/posts/relay.rs
```

- [ ] **Step 3: Create `outbound/posts/mod.rs` and aliases**

```rust
//! Outbound post command builders.

pub mod announcement;
pub mod approval;
pub mod existing;
pub mod props;
pub mod read;
pub mod read_ext;
pub mod relay;
```

In `outbound/mod.rs`, add `pub mod posts;` and these compatibility aliases:

```rust
pub use posts::announcement as posts_announcement;
pub use posts::approval as posts_approval;
pub use posts::existing as posts_existing;
pub use posts::props as post_props;
pub use posts::read as posts_read;
pub use posts::read_ext as posts_read_ext;
pub use posts::relay as posts_relay;
```

- [ ] **Step 4: Run #15 GREEN**

```bash
scripts/ddd-structure-gate.sh 15
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #15**

Commit message: `refactor(im): 拆分 outbound posts 子域`

## Task 9: Issue #16 outbound/engagement Split

**Files:** `vote_score*`, `quick_reply`, `urgent`, `template_received`, `outbound/engagement/mod.rs`, `outbound/mod.rs`.

- [ ] **Step 1: Run #16 RED**

```bash
scripts/ddd-structure-gate.sh 16
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/outbound/engagement
git mv crates/helix-im/src/outbound/vote_score.rs crates/helix-im/src/outbound/engagement/vote_score.rs
git mv crates/helix-im/src/outbound/vote_score_tests.rs crates/helix-im/src/outbound/engagement/vote_score_tests.rs
git mv crates/helix-im/src/outbound/quick_reply.rs crates/helix-im/src/outbound/engagement/quick_reply.rs
git mv crates/helix-im/src/outbound/urgent.rs crates/helix-im/src/outbound/engagement/urgent.rs
git mv crates/helix-im/src/outbound/template_received.rs crates/helix-im/src/outbound/engagement/template_received.rs
```

- [ ] **Step 3: Create `outbound/engagement/mod.rs` and aliases**

```rust
//! Outbound engagement command builders.

pub mod quick_reply;
pub mod template_received;
pub mod urgent;
pub mod vote_score;
```

In `outbound/mod.rs`, add `pub mod engagement;` and these compatibility aliases:

```rust
pub use engagement::quick_reply;
pub use engagement::template_received;
pub use engagement::urgent;
pub use engagement::vote_score;
```

- [ ] **Step 4: Run #16 GREEN**

```bash
scripts/ddd-structure-gate.sh 16
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #16**

Commit message: `refactor(im): 拆分 outbound engagement 子域`

## Task 10: Issue #17 outbound/user_misc Split

**Files:** `user_misc*`, `outbound/user_misc/mod.rs`, `outbound/mod.rs`.

- [ ] **Step 1: Run #17 RED**

```bash
scripts/ddd-structure-gate.sh 17
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/outbound/user_misc
git mv crates/helix-im/src/outbound/user_misc.rs crates/helix-im/src/outbound/user_misc/core.rs
git mv crates/helix-im/src/outbound/user_misc_helpers.rs crates/helix-im/src/outbound/user_misc/helpers.rs
git mv crates/helix-im/src/outbound/user_misc_tests.rs crates/helix-im/src/outbound/user_misc/core_tests.rs
```

- [ ] **Step 3: Create `outbound/user_misc/mod.rs` and aliases**

```rust
//! Outbound user and miscellaneous command builders.

pub mod core;
mod helpers;

pub use core::*;
```

In `outbound/mod.rs`, add `pub mod user_misc;` and keep helper visibility private unless compile errors require `pub(crate)`.

- [ ] **Step 4: Run #17 GREEN**

```bash
scripts/ddd-structure-gate.sh 17
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #17**

Commit message: `refactor(im): 拆分 outbound user_misc 子域`

## Task 11: Issue #18 ws/parser Migration

**Files:** `parser.rs`, `parser_post_props.rs`, `ws/parser/mod.rs`, `ws/mod.rs`, `lib.rs`, `structure-gate.sh`, `.claude/rules/code-structure-ddd.md`.

- [ ] **Step 1: Run #18 RED**

```bash
scripts/ddd-structure-gate.sh 18
```

- [ ] **Step 2: Move files**

```bash
mkdir -p crates/helix-im/src/ws/parser
git mv crates/helix-im/src/parser.rs crates/helix-im/src/ws/parser/core.rs
git mv crates/helix-im/src/parser_post_props.rs crates/helix-im/src/ws/parser/post_props.rs
```

- [ ] **Step 3: Create `ws/parser/mod.rs`**

```rust
//! WebSocket inbound frame parser.

pub mod core;
mod post_props;

pub use core::*;
```

- [ ] **Step 4: Update module declarations and whitelist mirrors**

In `ws/mod.rs`, add:

```rust
pub mod parser;
```

In `lib.rs`, remove top-level `pub mod parser;` and `mod parser_post_props;`, then add:

```rust
pub use ws::parser;
```

In `scripts/structure-gate.sh`, replace `crates/helix-im/src/parser.rs 430` with `crates/helix-im/src/ws/parser/core.rs 430`.

In `.claude/rules/code-structure-ddd.md`, replace `helix-im/src/parser.rs` with `helix-im/src/ws/parser/core.rs`.

- [ ] **Step 5: Run #18 GREEN**

```bash
scripts/ddd-structure-gate.sh 18
cargo check -p helix-im
cargo test -p helix-im
./scripts/structure-gate.sh
```

- [ ] **Step 6: Commit #18**

Commit message: `refactor(im): 迁移 ws parser 到入站 ACL 管道`

## Task 12: Issue #19 helix-im CLAUDE.md/AGENTS.md Rewrite

**Files:** `crates/helix-im/CLAUDE.md`, `crates/helix-im/AGENTS.md`.

- [ ] **Step 1: Run #19 RED**

```bash
scripts/ddd-structure-gate.sh 19
```

Expected: FAIL because the docs do not yet describe every final bounded context with 职责/怎么用/依赖.

- [ ] **Step 2: Rewrite `crates/helix-im/CLAUDE.md`**

Add a section `## src 限界上下文地图` containing exactly these context headings and three bullets under each: `职责`, `怎么用`, `依赖`.

Required headings:

```markdown
### channel/
### send/
### sync/
### query/
### todo/
### outbound/
### ws/
### acl/
```

- [ ] **Step 3: Mirror to AGENTS.md**

```bash
cp crates/helix-im/CLAUDE.md crates/helix-im/AGENTS.md
```

- [ ] **Step 4: Run #19 GREEN**

```bash
scripts/ddd-structure-gate.sh 19
diff -u crates/helix-im/CLAUDE.md crates/helix-im/AGENTS.md
./scripts/structure-gate.sh
```

- [ ] **Step 5: Commit #19**

Commit message: `docs(im): 更新 helix-im 限界上下文地图`

## Task 13: Issue #20 coverage.sh MATCH_KEY Review

**Files:** `scripts/coverage.sh`, `docs/review/coverage-ddd-match-key-review.md`.

**Interfaces:**
- Consumes: `scripts/coverage.sh --phase <file>`
- Produces: documented conclusion on whether `MATCH_KEY="${FILE#crates/helix-im/src/}"` remains semantically correct after deeper directories

- [ ] **Step 1: Run #20 RED**

```bash
scripts/ddd-structure-gate.sh 20
```

Expected: FAIL because `docs/review/coverage-ddd-match-key-review.md` does not exist.

- [ ] **Step 2: Inspect MATCH_KEY flow**

Read `scripts/coverage.sh` and confirm:

```bash
MATCH_KEY="${FILE#crates/helix-im/src/}"
LINE="$(grep -F "$MATCH_KEY" /tmp/helix-cov-summary.txt || true)"
```

The key intentionally preserves nested paths under `src/`, so `crates/helix-im/src/channel/gate.rs` becomes `channel/gate.rs`, matching llvm-cov summary filenames.

- [ ] **Step 3: Write review document**

Create `docs/review/coverage-ddd-match-key-review.md`:

```markdown
# coverage.sh MATCH_KEY Review After helix-im DDD Split

## Scope

Reviewed `scripts/coverage.sh --phase <file>` after the #9-#18 directory split.

## Code Path

`MATCH_KEY="${FILE#crates/helix-im/src/}"` strips only the crate source prefix and preserves any deeper directory components. The result is passed to `grep -F "$MATCH_KEY"` against `cargo llvm-cov --summary-only` output.

## 目录变深影响

The DDD split makes paths deeper, for example `crates/helix-im/src/channel/gate.rs` becomes `channel/gate.rs`. This is still the shape emitted by llvm-cov summary for crate-local source files, so preserving nested components is correct.

## 结论

No `coverage.sh` code change is required. Mechanical replacement would be riskier than keeping the current prefix-strip behavior because the current behavior already handles arbitrary depth below `crates/helix-im/src/`.

## Verification

Structural check: `scripts/ddd-structure-gate.sh 20`.
Optional runtime check when `cargo-llvm-cov` is installed: `scripts/coverage.sh --phase crates/helix-im/src/channel/gate.rs`.
```

- [ ] **Step 4: Run #20 GREEN**

```bash
scripts/ddd-structure-gate.sh 20
```

If `cargo llvm-cov --version` succeeds locally, also run:

```bash
scripts/coverage.sh --phase crates/helix-im/src/channel/gate.rs
```

Expected: the command must not fail with "未在覆盖率报告找到 channel/gate.rs". A coverage percentage failure is acceptable for this review because #20 verifies path matching semantics, not coverage quality. If cargo-llvm-cov is missing, record that the optional runtime check was skipped.

- [ ] **Step 5: Commit #20**

Commit message: `docs(im): 复核 coverage MATCH_KEY 目录层级语义`

## Final Verification

- [ ] Run full structural gate:

```bash
scripts/ddd-structure-gate.sh all
```

- [ ] Run dual-end check:

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
```

- [ ] Run helix-im tests:

```bash
cargo test -p helix-im
```

- [ ] Run core runtime-dependency guard:

```bash
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

Expected: no output.

- [ ] Run project gates:

```bash
./scripts/structure-gate.sh
./scripts/gate.sh
```

- [ ] Add terminal line to `SESSION.md` §0:

```text
✅ DONE helix-im DDD 包分层 #9-#20 @<timestamp> | commit d221454..<HEAD> | 12/12 issues | main
```

- [ ] Update GitLab labels for each issue that is actually complete:

```bash
glab issue update <id> --label status::review --unlabel ready-for-agent
```

Only do this after that issue's RED/GREEN and verification evidence exists.
