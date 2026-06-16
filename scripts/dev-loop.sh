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
# env（均有默认）：HELIX_WS_URL/API_BASE/COOKIE_ID/COMPANY_ID/DB/GOLOG/FOCUS/RUST_LOG
set -uo pipefail
cd "$(dirname "$0")/.."

DUR="${1:-12}"
GOLOG="${GOLOG:-/Users/mac28/workspace/mmlog/mattermost.log}"
# FOCUS = 本轮调试关注的功能关键词（grep -E 正则）。换功能只改这一个。
FOCUS="${FOCUS:-increment|channel|sync|post|read|grpc}"
export HELIX_WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
export HELIX_API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
export HELIX_COOKIE_ID="${HELIX_COOKIE_ID:-5ffd465e263864787b87aa01}"
export HELIX_COMPANY_ID="${HELIX_COMPANY_ID:-64118eebd2b665246b7880eb}"
export HELIX_DB="${HELIX_DB:-/tmp/helix_devloop_$(date +%s).db}"
export HELIX_DUMP_FRAMES=1
export RUST_LOG="${RUST_LOG:-info,helix_im=debug,helix_driver_native=info}"
HLOG="/tmp/helix_devloop_$(date +%s).log"
strip() { sed -E 's/\x1b\[[0-9;]*m//g'; }

# ── [0] 自动自检/恢复：Go 服务端在线 + Go 日志可读 ──────────────────────
echo "▶ [0/4] 环境自检/恢复"
if ! lsof -nP -iTCP:8065 -sTCP:LISTEN -t >/dev/null 2>&1; then
  echo "  ⛔ Go 服务端 :8065 未监听 — 请先启动 mattermost server（GoLand run / make run-server），再重跑"
  exit 2
fi
[ -r "$GOLOG" ] || { echo "  ⚠️ Go 日志不可读: $GOLOG（设 GOLOG=... 指向真实日志）— ② 层将为空"; }
echo "  ✅ Go :8065 在线；日志 $GOLOG"

# ── [1] 构建 ────────────────────────────────────────────────────────────
echo "▶ [1/4] 构建 host-cli"
cargo build -p helix-host-cli --quiet || { echo "  ⛔ 构建失败"; exit 1; }

# ── [2] 跑 helix（记 Go 日志基线，跑 N 秒）──────────────────────────────
echo "▶ [2/4] Go 日志基线 + 跑 helix ${DUR}s（FOCUS='${FOCUS}'）"
GOBASE=$(wc -l < "$GOLOG" 2>/dev/null || echo 0)
timeout "$DUR" ./target/debug/helix-host-cli > "$HLOG" 2>&1

# ── [3] 三层关联 ────────────────────────────────────────────────────────
echo ""
echo "════════ ① 代码层：helix Effect / emit / 握手 / 警告（FOCUS + 通用）════════"
grep -iE "handshake|DomainEvent emit|disconnected|connection|warn|error|${FOCUS}" "$HLOG" \
  | grep -ivE "RAW WS inbound" | strip | sed -E 's/^[0-9TZ:.+ -]+//' | head -18

echo ""
echo "════════ ② 日志层：Go 日志 delta（FOCUS + 错误/grpc/close）════════"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "${FOCUS}|error|grpc|fail|panic|close|publish" \
  | grep -oE '"caller":"[^"]*"|"msg":"[^"]*"|"error":"[^"]*"|"method":"[^"]*"' \
  | paste -sd '|' - 2>/dev/null | tr '|' '\n' | sed 's/^/  /' | head -26
echo "  --- 关键原始错误行（截断）---"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "error|fail|panic|grpc service" | cut -c1-190 | sed 's/^/  /' | head -6

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
