#!/usr/bin/env bash
# branch-sync-gate.sh —— 分叉监控的**机器闸门**（替代 SESSION 散文便利贴，drift-review #2 收敛）。
#
# 病根（用户 2026-06-22 指出）：SESSION 的「落后 ≥5 就 rebase」只是散文提醒、无脚本无 hook →
# 没机制触发 → 并行开发几天就静默分叉（feat 一度落后 dev-zlc 17）。本脚本把它硬化成退出码闸门，
# 供人工或开局检查按需执行，不安装或恢复 Git pre-push hook。
#
# 不变量：当前分支落后集成分支（默认 dev-zlc）的 commit 数 ≥ 阈值 → FAIL（先 rebase/merge 再续）。
# 退出码：0 = ok（落后 < 阈值）/ 1 = 强制同步（落后 ≥ 阈值）/ 2 = 集成分支不存在（异常）。
# 输出可 grep 前缀：BRANCH-SYNC。
set -uo pipefail

INTEGRATION="${HELIX_INTEGRATION_BRANCH:-dev-zlc}"
THRESHOLD="${HELIX_BRANCH_SYNC_THRESHOLD:-5}"

cur="$(git rev-parse --abbrev-ref HEAD 2>/dev/null)"
if [ -z "$cur" ]; then echo "BRANCH-SYNC ✗ 取不到当前分支"; exit 2; fi

# 在集成分支本身 → 无所谓落后。
if [ "$cur" = "$INTEGRATION" ]; then
  echo "BRANCH-SYNC ✅ 当前即集成分支 ${INTEGRATION}，无分叉监控对象"
  exit 0
fi

if ! git rev-parse --verify --quiet "$INTEGRATION" >/dev/null; then
  echo "BRANCH-SYNC ⚠️ 集成分支 ${INTEGRATION} 不存在（设 HELIX_INTEGRATION_BRANCH 覆盖）—— 跳过"
  exit 2
fi

behind="$(git rev-list --count "HEAD..${INTEGRATION}" 2>/dev/null || echo 0)"
ahead="$(git rev-list --count "${INTEGRATION}..HEAD" 2>/dev/null || echo 0)"

echo "BRANCH-SYNC 当前 ${cur} vs 集成 ${INTEGRATION}：落后 ${behind} · 领先 ${ahead}（阈值 ${THRESHOLD}）"
if [ "$behind" -ge "$THRESHOLD" ]; then
  echo "BRANCH-SYNC 🔴 落后 ${behind} ≥ ${THRESHOLD} —— 先 \`git rebase ${INTEGRATION}\` 或 \`git merge ${INTEGRATION}\` 再续开发，别让架构分叉"
  exit 1
fi
echo "BRANCH-SYNC ✅ 落后 ${behind} < ${THRESHOLD}，无需同步"
exit 0
