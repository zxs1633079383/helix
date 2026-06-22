#!/usr/bin/env bash
# structure-gate.sh —— 代码结构约束闸门（.claude/rules/code-structure-ddd.md 的可执行护栏）
#
# 两道检查（与 gate.sh / review-gap.sh 同族，纳入 §4.7 drift-review lens）：
#   ① 行数闸：crates/**/src/**.rs 硬顶 300 行；存量债走 WHITELIST 豁免（只减不增——
#             改小到 ≤300 后请从 WHITELIST 删除该行，使其重新受闸）。新增超限文件 = 失败。
#   ② 镜像闸：每个 crates/*/ 必须有 CLAUDE.md + AGENTS.md 且内容完全一致（codex 镜像）。
#
# 退出码：0 = 通过 / 1 = 违规。输出可 grep 前缀：STRUCT-GATE。
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

MAX_LINES=300
fail=0

# ── 存量债白名单（src/ 已超 300 行，待拆 backlog；见 rule §1）。只减不增。 ──
# 行尾注释为当前行数（参考，闸门不读注释，只比对路径）。
WHITELIST=(
  "crates/helix-driver-native/src/engine_loop.rs"   # ~1119
  "crates/helix-im/src/module.rs"                    # ~1077（merge 后 dispatch 拆分中）
  "crates/helix-im/src/ws/registry.rs"              # ~779
  "crates/helix-im/src/acl/to_effect.rs"            # ~528
  "crates/helix-im/src/channel.rs"                  # ~499
  "crates/helix-driver-host/src/network.rs"         # ~475
  "crates/helix-driver-host/src/storage.rs"         # ~447
  "crates/helix-im/src/parser.rs"                   # ~393
  "crates/helix-im/src/state.rs"                    # ~378
  "crates/helix-core/src/effect.rs"                 # ~337
  "crates/helix-driver-native/src/event_sink.rs"    # ~317
  "crates/helix-driver-ffi/src/api.rs"              # ~307
)
is_whitelisted() {
  local f="$1"
  for w in "${WHITELIST[@]}"; do [ "$w" = "$f" ] && return 0; done
  return 1
}

echo "STRUCT-GATE ① 行数闸（src/**.rs ≤ ${MAX_LINES}，存量豁免 ${#WHITELIST[@]} 个）"
while IFS= read -r f; do
  lines=$(wc -l < "$f" | tr -d ' ')
  [ "$lines" -le "$MAX_LINES" ] && continue
  if is_whitelisted "$f"; then
    echo "STRUCT-GATE   · 豁免(存量债) $f = $lines 行 —— 改到时顺手拆，拆完移出 WHITELIST"
  else
    echo "STRUCT-GATE   ✗ 超限(新增) $f = $lines 行 > ${MAX_LINES} —— 拆分（many small files / mod.rs 只 re-export）"
    fail=1
  fi
done < <(find crates -path '*/src/*' -name '*.rs' -not -path '*/target/*' | sort)

echo "STRUCT-GATE ② 镜像闸（每 crates/*/ 须有 CLAUDE.md + AGENTS.md 且一致）"
for d in crates/*/; do
  c="${d}CLAUDE.md"; a="${d}AGENTS.md"
  if [ ! -f "$c" ]; then echo "STRUCT-GATE   ✗ 缺 $c"; fail=1; continue; fi
  if [ ! -f "$a" ]; then echo "STRUCT-GATE   ✗ 缺 ${a}（AGENTS.md 是 codex 镜像，cp 自 CLAUDE.md）"; fail=1; continue; fi
  if ! diff -q "$c" "$a" >/dev/null 2>&1; then
    echo "STRUCT-GATE   ✗ 不一致 ${c} ↔ ${a}（镜像必须完全相同）"; fail=1
  fi
done

if [ "$fail" -eq 0 ]; then
  echo "STRUCT-GATE ✅ PASS"
else
  echo "STRUCT-GATE 🔴 FAIL —— 见上方 ✗ 项"
fi
exit "$fail"
