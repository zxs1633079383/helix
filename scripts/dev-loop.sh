#!/usr/bin/env bash
# dev-loop.sh — 通用 sync-v2 真 Go 联调三测探针（代码 ↔ 日志 ↔ 结果）
#
# 不绑定任何具体功能。任意功能调试都用它：自检/恢复环境 → 跑 helix 真连 Go →
# 抓三层（helix Effect/emit ＋ Go 日志 delta ＋ helix 收帧）→ 按 FOCUS 关键词关联打印。
# 配套「loop Engine」驱动法见 .claude/skills/helix-loop-engine/SKILL.md（固定通用 prompt）。
#
# 用法：
#   scripts/dev-loop.sh [秒数]
#   FOCUS='read|post_read' scripts/dev-loop.sh 10        # 调已读功能
#   FOCUS='ping|heartbeat|close' scripts/dev-loop.sh 12  # 调心跳/断连
#   FOCUS='increment|channel' scripts/dev-loop.sh        # 调增量（默认）
#
# env（均有默认，从 helix.config.yaml 读取）：FOCUS/RUST_LOG/HELIX_CONFIG_PATH
set -uo pipefail
cd "$(dirname "$0")/.."

# ── 读配置（helix.config.yaml → HELIX_GO_PORT/GO_LOG/WS_URL 等）────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

DUR="${1:-12}"
GOLOG="${HELIX_GO_LOG}"
# FOCUS = 本轮调试关注的功能关键词（grep -E 正则）。换功能只改这一个。
FOCUS="${FOCUS:-increment|channel|sync|post|read}"
export HELIX_DB="${HELIX_DB:-/tmp/helix_devloop_$(date +%s).db}"
export RUST_LOG="${RUST_LOG:-info,helix_im=debug,helix_driver_native=info}"
HLOG="/tmp/helix_devloop_$(date +%s).log"
strip() { sed -E 's/\x1b\[[0-9;]*m//g'; }

# ── [0] 自动自检/恢复：Go 服务端在线 + Go 日志可读 ──────────────────────
echo "▶ [0/4] 环境自检/恢复"
if ! lsof -nP -iTCP:"$HELIX_GO_PORT" -sTCP:LISTEN -t >/dev/null 2>&1; then
  echo "  ⛔ Go 服务端 :$HELIX_GO_PORT 未监听 — ${HELIX_START_HINT}"
  exit 2
fi
[ -r "$GOLOG" ] || { echo "  ⚠️ Go 日志不可读: $GOLOG（设 HELIX_GO_LOG=... 指向真实日志）— ② 层将为空"; }
echo "  ✅ Go :$HELIX_GO_PORT 在线；日志 $GOLOG"

# ── [1] 构建 ────────────────────────────────────────────────────────────
echo "▶ [1/4] 构建 host-cli"
cargo build -p helix-host-cli --quiet || { echo "  ⛔ 构建失败"; exit 1; }

# ── [2] 跑 helix（记 Go 日志基线，跑 N 秒）──────────────────────────────
echo "▶ [2/4] Go 日志基线 + 跑 helix ${DUR}s（FOCUS='${FOCUS}'）"
GOBASE=$(wc -l < "$GOLOG" 2>/dev/null || echo 0)
timeout "$DUR" ./target/debug/helix-host-cli --config "$HELIX_CONFIG_PATH" > "$HLOG" 2>&1

# ── [3] 三层关联 ────────────────────────────────────────────────────────
echo ""
echo "════════ ① 代码层：helix Effect / emit / 握手 / 警告（FOCUS + 通用）════════"
grep -iE "handshake|DomainEvent emit|disconnected|connection|warn|error|${FOCUS}" "$HLOG" \
  | grep -ivE "RAW WS inbound" | strip | sed -E 's/^[0-9TZ:.+ -]+//' | head -18

echo ""
echo "════════ ② 日志层：Go 日志 delta（FOCUS + 错误/close）════════"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "${FOCUS}|error|fail|panic|close|publish" \
  | grep -oE '"caller":"[^"]*"|"msg":"[^"]*"|"error":"[^"]*"|"method":"[^"]*"' \
  | paste -sd '|' - 2>/dev/null | tr '|' '\n' | sed 's/^/  /' | head -26
echo "  --- 关键原始错误行（截断）---"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "error|fail|panic" | cut -c1-190 | sed 's/^/  /' | head -6

echo ""
echo "════════ ③ 结果层：helix WS 收帧（按 action）════════"
grep -oE '"action":"[a-z_]+"' "$HLOG" 2>/dev/null | sort | uniq -c | sed 's/^/  /'
FOCUS_FRAMES=$(grep -ciE "\"action\":\"($FOCUS)" "$HLOG" 2>/dev/null); FOCUS_FRAMES=${FOCUS_FRAMES:-0}
echo "  → FOCUS 相关帧: ${FOCUS_FRAMES}"

echo ""
echo "════════ 产物（供深挖）════════"
echo "  helix 全日志 : $HLOG"
echo "  Go 日志 delta: tail -n +$((GOBASE+1)) $GOLOG"
echo "  换功能调试   : FOCUS='<关键词>' scripts/dev-loop.sh"
