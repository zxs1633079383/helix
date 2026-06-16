#!/usr/bin/env bash
# review-gap.sh — 架构 review 收敛闸门
#
# 算 HEAD 距上次 agent/workflow 架构 review 的 commit 距离（gap）。
# gap ≥ 阈值(5) → 强制派发 Workflow({name:"architecture-drift-review"}) 收敛后才能继续 feature 开发。
# 规则：.claude/rules/agent-review-cadence.md   台账：docs/review/workflow-review-ledger.md
#
# 退出码：0=ok（gap<阈值） / 1=强制 review（gap≥阈值） / 2=ledger 异常
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"
LEDGER="docs/review/workflow-review-ledger.md"
THRESHOLD=5

[ -f "$LEDGER" ] || { echo "⛔ 缺 $LEDGER"; exit 2; }

LAST=$(grep -m1 '^last_reviewed_commit:' "$LEDGER" | awk '{print $2}')
[ -n "$LAST" ] || { echo "⛔ ledger 缺 last_reviewed_commit"; exit 2; }

if ! git cat-file -e "${LAST}^{commit}" 2>/dev/null; then
  echo "⛔ last_reviewed_commit=$LAST 不在 git 历史里（被 rebase/squash？）—— 手动校正 ledger"; exit 2
fi

GAP=$(git rev-list --count "${LAST}..HEAD")
HEAD_SHORT=$(git rev-parse --short HEAD)

echo "last_reviewed_commit: $LAST"
echo "HEAD:                 $HEAD_SHORT"
echo "gap:                  $GAP / threshold: $THRESHOLD"

if [ "$GAP" -ge "$THRESHOLD" ]; then
  echo "🔴 强制 review：gap($GAP) ≥ 阈值($THRESHOLD)"
  echo "   → 派发 Workflow({name:\"architecture-drift-review\"}) 收敛架构后，更新 ledger，方可继续 feature 开发"
  exit 1
fi

echo "🟢 ok：剩 $((THRESHOLD - GAP)) 个 commit 触发强制架构 review"
exit 0
