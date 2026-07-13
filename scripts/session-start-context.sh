#!/usr/bin/env bash

# Keep startup context compact: current blockers plus the canonical UC contract scope.
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$ROOT" || exit 0

UC_DOC="crates/helix-im/docs/前端UC投影指令与数据清单-最新代码分析-2026-07-09.md"

printf '%s\n' '🧭 Helix 开局：① 读 docs/ROADMAP.md §二 P0 活账 ② 读 docs/harness/OPERATING.md 触发表 ③ 读前端 UC 投影真源 ④ 闸门 scripts/gate.sh · 结构 scripts/structure-gate.sh · review scripts/review-gap.sh'

OPEN=$(awk '
  BEGIN { in_p0=0 }
  /^\*\*P0（M2 收官阻塞）\*\*/ { in_p0=1; next }
  /^\*\*P1（/ { in_p0=0 }
  in_p0 && /^- \[ \]/ { print }
' docs/ROADMAP.md 2>/dev/null | head -6)

if [ -n "$OPEN" ]; then
  printf '── 未完 P0（仅 P0 段）──\n%s\n' "$OPEN"
fi

if [ -f "$UC_DOC" ]; then
  printf '── 前端 UC 投影上下文真源（开局必读）──\n'
  sed -n '1,16p' "$UC_DOC"
else
  printf '⚠️ SessionStart 缺少 UC 真源：%s\n' "$UC_DOC"
fi
