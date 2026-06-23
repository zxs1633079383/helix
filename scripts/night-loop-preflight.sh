#!/usr/bin/env bash
# night-loop-preflight.sh — 夜间 goal-loop 一键预检 + 沙箱创建 + NIGHT_GOAL.md 生成
# 用法：
#   bash scripts/night-loop-preflight.sh <task-slug> [--arm]      # 预检+建 worktree(+装 deny)
#   bash scripts/night-loop-preflight.sh <task-slug> --disarm     # 还原 deny 设置
#   bash scripts/night-loop-preflight.sh <task-slug> --teardown   # 删 worktree + branch（merge 后清理）
# 配套手册：docs/harness/night-loop-playbook.md
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

SLUG="${1:-}"
MODE="${2:-}"
[ -z "$SLUG" ] && { echo "⛔ 用法: night-loop-preflight.sh <task-slug> [--arm|--disarm|--teardown]" >&2; exit 1; }

WT="worktrees/night-$SLUG"
BRANCH="night/$SLUG"
DENY_TPL=".claude/night-loop.settings.local.json"
DENY_ACTIVE=".claude/settings.local.json"
DENY_BAK=".claude/settings.local.json.pre-night.bak"

arm_deny() {
  [ -f "$DENY_TPL" ] || { echo "⚠️ 缺 $DENY_TPL，跳过 deny 装载" >&2; return 0; }
  if [ -f "$DENY_ACTIVE" ] && [ ! -f "$DENY_BAK" ]; then cp "$DENY_ACTIVE" "$DENY_BAK"; echo "🔒 已备份原 settings.local.json → $DENY_BAK"; fi
  cp "$DENY_TPL" "$DENY_ACTIVE"
  echo "🛡  deny 已装载（push/merge/reset/rm -rf/cargo clean/gh 已封）。晨间还原：--disarm"
}
disarm_deny() {
  if [ -f "$DENY_BAK" ]; then mv "$DENY_BAK" "$DENY_ACTIVE"; echo "🔓 已还原原 settings.local.json"
  elif [ -f "$DENY_ACTIVE" ]; then rm "$DENY_ACTIVE"; echo "🔓 已移除夜间 deny 设置（原本无 settings.local.json）"
  else echo "ℹ️ 无 deny 设置可还原"; fi
}

case "$MODE" in
  --disarm)  disarm_deny; exit 0 ;;
  --teardown)
    git worktree remove --force "$WT" 2>/dev/null && echo "🧹 已删 worktree $WT" || echo "ℹ️ 无 worktree $WT"
    git branch -D "$BRANCH" 2>/dev/null && echo "🧹 已删 branch $BRANCH" || echo "ℹ️ 无 branch $BRANCH"
    git worktree prune
    exit 0 ;;
esac

# 1) df 预检（ENOSPC 防御）；可移植：解析 df -k 第4列(KB)→GB。阈值可用 MIN_GB 覆盖（默认 10）
MIN_GB="${MIN_GB:-10}"
AVAIL_GB="$(df -k ~ 2>/dev/null | awk 'NR>1{print int($4/1024/1024); exit}')"
[ -z "$AVAIL_GB" ] && AVAIL_GB=0
[ "$AVAIL_GB" -lt "$MIN_GB" ] && { echo "⛔ 磁盘可用 ${AVAIL_GB}GB < ${MIN_GB}GB，拒绝创建 worktree（ENOSPC 防御）" >&2; exit 1; }
echo "✅ 磁盘可用 ${AVAIL_GB}GB"

# 2) 当前分支 guard（不在 main/dev-zlc 上裸跑，且 worktrees 已被 .gitignore）
CUR="$(git rev-parse --abbrev-ref HEAD)"
echo "ℹ️ 主仓当前分支：$CUR"
grep -qE '^/?worktrees/?$' .gitignore || echo "⚠️ .gitignore 未含 /worktrees/，建议补上" >&2

# 3) 建 worktree + branch（已存在则复用）
if git worktree list --porcelain | grep -q "worktree $REPO_ROOT/$WT"; then
  echo "ℹ️ worktree $WT 已存在，复用"
else
  git worktree add "$WT" -b "$BRANCH"
  echo "✅ 已建 worktree $WT（branch $BRANCH）"
fi

# 4) 生成 NIGHT_GOAL.md（不覆盖已有）
GOAL="$WT/NIGHT_GOAL.md"
if [ -f "$GOAL" ]; then
  echo "ℹ️ $GOAL 已存在，保留不覆盖"
else
  cat > "$GOAL" <<EOF
# 夜间任务目标 — $SLUG
任务：<一句话，填我>
启动：$(date '+%Y-%m-%dT%H:%M:%S%z') | 分支：$BRANCH | worktree：$WT/

## 守护约束（GUARD — 改动违反任一立即 abort 写 BLOCKED.md）
- 不违反 docs/GOAL.md §1 设计理念不变量 / §2 硬约束
- 不违反 HX-C001..C015 在册护栏

## 成功条件（SUCCESS — 全部满足才算 done，机器可验证）
- [ ] \`bash scripts/gate.sh\` 退出码 0
- [ ] \`bash scripts/structure-gate.sh\` 退出码 0
- [ ] \`cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi\` 通过
- [ ] \`cargo test -p helix-core\` 通过
- [ ] \`git status --porcelain\` 为空
- [ ] <任务专属验收，填我>

## 停止条件（STOP — 命中任一立刻停，写 PARTIAL + REPORT.md）
- 迭代 ≥ 12 tick 未达 SUCCESS
- 连续 3 tick git HEAD 无变化（无进展）
- 累计输出 token ≥ 400k
- 命中 BLOCKED（需跨仓 / 需人决策 / 测试逻辑性失败）

## 禁区（NEVER）
- 不碰 main / dev-zlc；不 push / merge / reset --hard / cargo clean
- 不改根目录黑名单 SESSION.md / CLAUDE.md / README.md；不创建或改 GOAL.md
- 不删非本任务创建的文件
EOF
  echo "✅ 已生成 $GOAL（请填「任务」与「任务专属验收」两行）"
fi

# 5) 可选装 deny
[ "$MODE" = "--arm" ] && arm_deny

cat <<EOF

—— 下一步 ——
1. 编辑 $GOAL，填实「任务」与专属 SUCCESS 验收
2. 派 autonomous agent / Workflow 跑 goal loop（prompt 要点）：
     读 $GOAL + docs/GOAL.md（守护）→ 循环执行 → 每 tick 跑 SUCCESS 命令链
     → 命中 STOP 写 $WT/REPORT.md + SESSION.md §0 终态行；预算 +400k
3. 晨间：看 $WT/REPORT.md → git diff main..$BRANCH 审 → 自行决定 merge
4. 清理：bash scripts/night-loop-preflight.sh $SLUG --disarm   # 还原 deny
         bash scripts/night-loop-preflight.sh $SLUG --teardown # merge 后删 worktree
EOF
