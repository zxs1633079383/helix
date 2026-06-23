#!/usr/bin/env bash
# branch-identity.sh —— 「我在哪个分支」一键自陈（HX-C014 配套）。
#
# 病根（用户 2026-06-22 反复指出）：单分支 + worktree + 手工 merge/删除几天后，
# 主对话与用户都迷失「当前在哪个分支 / 有没有 upstream / 哪些分支该删」——
# 同一句「你在哪个分支」一天问 ≥6 次。本脚本把答案固化成一条命令、可贴进 SESSION §0。
#
# 与 branch-sync-gate.sh 的分工：
#   - branch-sync-gate.sh = 闸门（落后集成分支 ≥阈值 → 退出码 1 FAIL）。
#   - branch-identity.sh   = 自陈（只读打印「在哪/领先落后/worktree/可删候选」，恒退出 0）。
#
# 输出可 grep 前缀：BRANCH-ID。`--session` 输出可直接粘进 SESSION §0 的单行 🧭 摘要。
set -uo pipefail

INTEGRATION="${HELIX_INTEGRATION_BRANCH:-dev-zlc}"
MAIN="${HELIX_MAIN_BRANCH:-main}"
MODE="${1:-full}"   # full | --session

short() { git rev-parse --short "$1" 2>/dev/null || echo "????"; }
ab() { # ahead behind: $1=base ref → 打印 "↑ahead ↓behind"
  local base="$1"
  if git rev-parse --verify --quiet "$base" >/dev/null; then
    local a b
    a="$(git rev-list --count "${base}..HEAD" 2>/dev/null || echo 0)"
    b="$(git rev-list --count "HEAD..${base}" 2>/dev/null || echo 0)"
    echo "↑${a} ↓${b}"
  else
    echo "（无 ${base}）"
  fi
}

cur="$(git rev-parse --abbrev-ref HEAD 2>/dev/null)"
[ -z "$cur" ] && { echo "BRANCH-ID ✗ 取不到当前分支（非 git 仓？）"; exit 0; }
cursha="$(short HEAD)"
upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo '无 upstream')"

# ── --session 单行摘要（贴 SESSION §0）──────────────────────────────
if [ "$MODE" = "--session" ]; then
  stamp="$(date '+%Y-%m-%d %H:%M %z')"
  printf '🧭 BRANCH-ID @%s | 当前 `%s`@%s ↑↓vs%s:%s · upstream:%s | worktrees:' \
    "$stamp" "$cur" "$cursha" "$MAIN" "$(ab "$MAIN")" "$upstream"
  git worktree list --porcelain 2>/dev/null \
    | awk '/^worktree /{p=$2} /^branch /{gsub("refs/heads/","",$2); printf " %s[%s]", p, $2}' \
    | sed "s|$(git rev-parse --show-toplevel 2>/dev/null)|.|g"
  echo ""
  exit 0
fi

# ── full 自陈块 ────────────────────────────────────────────────────
echo "BRANCH-ID ═══════════════ 我在哪 ═══════════════"
echo "BRANCH-ID 当前分支 : ${cur}  @${cursha}"
echo "BRANCH-ID upstream : ${upstream}  ($(ab "$upstream" 2>/dev/null))"
echo "BRANCH-ID vs ${MAIN}        : $(ab "$MAIN")"
echo "BRANCH-ID vs ${INTEGRATION} (集成) : $(ab "$INTEGRATION")"

echo "BRANCH-ID ── worktrees ──"
git worktree list 2>/dev/null | sed 's/^/BRANCH-ID   /'

# 可删候选：本地分支已并入 main 且非当前/集成/主分支
echo "BRANCH-ID ── 可删候选（已并入 ${MAIN}，非 worktree 占用）──"
wt_branches="$(git worktree list --porcelain 2>/dev/null | awk '/^branch /{gsub("refs/heads/","",$2); print $2}')"
found_stale=0
if git rev-parse --verify --quiet "$MAIN" >/dev/null; then
  while IFS= read -r b; do
    [ -z "$b" ] && continue
    case "$b" in "$MAIN"|"$INTEGRATION"|"$cur") continue;; esac
    echo "$wt_branches" | grep -qx "$b" && continue   # worktree 占用，不能删
    # 该分支所有 commit 都已在 main → 合并完毕可删
    if [ "$(git rev-list --count "${MAIN}..${b}" 2>/dev/null || echo 1)" = "0" ]; then
      echo "BRANCH-ID   🗑️  ${b}  (已全部并入 ${MAIN} → 可 \`git branch -d ${b}\`)"
      found_stale=1
    fi
  done <<< "$(git for-each-ref --format='%(refname:short)' refs/heads/)"
fi
[ "$found_stale" = "0" ] && echo "BRANCH-ID   （无——所有本地分支要么未并入、要么 worktree 占用）"

echo "BRANCH-ID ═══════════════════════════════════════"
echo "BRANCH-ID 提示：贴进 SESSION §0 用 \`scripts/branch-identity.sh --session\`"
exit 0
