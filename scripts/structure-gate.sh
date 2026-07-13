#!/usr/bin/env bash
# structure-gate.sh —— 代码结构约束闸门
#
# 检查：crates/**/src/**.rs 硬顶 300 行；存量债走 WHITELIST 豁免（只减不增——
# 改小到 ≤300 后请从 WHITELIST 删除该行，使其重新受闸）。新增超限文件 = 失败。
#
# 退出码：0 = 通过 / 1 = 违规。输出可 grep 前缀：STRUCT-GATE。
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

MAX_LINES=300
fail=0
CHANGED_SINCE=""
if [ "${1:-}" = "--changed-since" ]; then
  CHANGED_SINCE="${2:?--changed-since 需要基准 commit}"
fi

# ── 存量债白名单（src/ 已超 300 行，待拆 backlog；见 rule §1）：`path baseline_lines` ──
# 🔒 机器强制「只减不增」（drift-review wf_efcf1ca4-759 #1/#7 后门收敛）：闸门**读 baseline**——
#    当前行数 > baseline 即 FAIL（baseline = 锁定时的当前行数，红线 = 不许再涨）。
#    拆小到 ≤300 后从此表删除该行 → 重新受 300 硬顶。基线先锁(A)、实际拆分回正记 ROADMAP(B)。
# ⚠️ 后门第二形态（本次 drift-review 复发收敛）：**把 baseline 数字往大改 == 新增超限 == 等价绕 FAIL**。
#    超基线 → 唯一合法解是「测试外提 sibling / 拆文件」把行数压回 ≤baseline；**绝不允许上调 baseline 抹掉 FAIL**。
#    baseline 只允许往**小**改（拆分回正后锁更低值）。范本：state.rs 390→测试外提 state_tests.rs→340（本次）。
WHITELIST=(
  "crates/helix-im/src/module.rs 423"
  "crates/helix-im/src/ws/registry.rs 783"
  "crates/helix-driver-ffi/src/api.rs 733"
  "crates/helix-im/src/channel/gate.rs 579"
  "crates/helix-driver-host/src/storage.rs 560"
  "crates/helix-driver-host/src/network.rs 463"
  "crates/helix-core/src/effect.rs 452"
  "crates/helix-im/src/ws/parser/core.rs 430"
  "crates/helix-driver-native/src/event_sink.rs 321"
)
# 命中白名单 → stdout 回传 baseline + return 0；未命中 → return 1。
whitelist_baseline() {
  local f="$1"
  for entry in "${WHITELIST[@]}"; do
    if [ "${entry% *}" = "$f" ]; then echo "${entry##* }"; return 0; fi
  done
  return 1
}

if [ -n "$CHANGED_SINCE" ]; then
  echo "STRUCT-GATE 增量行数闸（${CHANGED_SINCE}..HEAD，新增 ≤ ${MAX_LINES}，存量只减不增）"
  while IFS= read -r f; do
    [ -f "$f" ] || continue
    lines=$(wc -l < "$f" | tr -d ' ')
    if git cat-file -e "${CHANGED_SINCE}:${f}" 2>/dev/null; then
      base_lines=$(git show "${CHANGED_SINCE}:${f}" | wc -l | tr -d ' ')
    else
      base_lines=0
    fi
    if [ "$lines" -le "$MAX_LINES" ] || { [ "$base_lines" -gt "$MAX_LINES" ] && [ "$lines" -le "$base_lines" ]; }; then
      continue
    fi
    echo "STRUCT-GATE   ✗ 增量超限 $f = $lines 行（基准 ${base_lines}）"
    fail=1
  done < <(git diff --name-only --diff-filter=ACMR "$CHANGED_SINCE"..HEAD -- 'crates/*/src/**/*.rs')
else
  echo "STRUCT-GATE 行数闸（src/**.rs ≤ ${MAX_LINES}，存量豁免 ${#WHITELIST[@]} 个·基线只减不增）"
while IFS= read -r f; do
  lines=$(wc -l < "$f" | tr -d ' ')
  [ "$lines" -le "$MAX_LINES" ] && continue
  if base=$(whitelist_baseline "$f"); then
    if [ "$lines" -gt "$base" ]; then
      echo "STRUCT-GATE   ✗ 存量债增长 $f = $lines 行 > 基线 ${base} —— 违『只减不增』，拆分回正或收口到基线"
      fail=1
    else
      echo "STRUCT-GATE   · 豁免(存量债) $f = $lines 行（≤基线 ${base}，只减不增 OK）"
    fi
  else
    echo "STRUCT-GATE   ✗ 超限(新增) $f = $lines 行 > ${MAX_LINES} —— 拆分（many small files / mod.rs 只 re-export）"
    fail=1
  fi
done < <(find crates -path '*/src/*' -name '*.rs' -not -path '*/target/*' | sort)
fi

if [ "$fail" -eq 0 ]; then
  echo "STRUCT-GATE ✅ PASS"
else
  echo "STRUCT-GATE 🔴 FAIL —— 见上方 ✗ 项"
fi
exit "$fail"
