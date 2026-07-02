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
  echo "DDD-STRUCT   . $*"
}

require_file() {
  local path="$1"
  [ -f "$path" ] && mark_ok "exists $path" || mark_fail "missing $path"
}

forbid_file() {
  local path="$1"
  [ ! -e "$path" ] && mark_ok "removed $path" || mark_fail "old path still exists $path"
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
