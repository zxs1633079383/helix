#!/usr/bin/env bash
# dev-loop.sh — sync-v2 真 Go 联调三测闭环（代码 → 日志 → 结果）
#
# 一次运行：构建 host-cli → 真连 Go 跑 N 秒（dump 入站帧）→ 抓 Go 日志 delta →
# 把三层关联打印出来，便于「定位 → 修 → 复验」闭环：
#   ① 代码层：helix 吐了什么 Effect（hello/increment HTTP/emit）
#   ② 日志层：Go RunIncrementWorker 干了什么（worker / parallel load / grpc / error）
#   ③ 结果层：helix WS 实际收到哪些 increment_channel/_end 帧
#
# 用法：
#   scripts/dev-loop.sh                          # 默认 12s
#   HELIX_COMPANY_ID=<cid> scripts/dev-loop.sh 15   # 带 companyId 跑 15s
#
# 环境变量（均有默认）：
#   HELIX_WS_URL / HELIX_API_BASE / HELIX_COOKIE_ID / HELIX_COMPANY_ID / HELIX_DB
#   GOLOG（Go 服务端日志，默认 /Users/mac28/workspace/mmlog/mattermost.log）
set -uo pipefail
cd "$(dirname "$0")/.."

DUR="${1:-12}"
GOLOG="${GOLOG:-/Users/mac28/workspace/mmlog/mattermost.log}"
export HELIX_WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
export HELIX_API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
export HELIX_COOKIE_ID="${HELIX_COOKIE_ID:-5ffd465e263864787b87aa01}"
export HELIX_DB="${HELIX_DB:-/tmp/helix_devloop_$(date +%s).db}"
export HELIX_DUMP_FRAMES=1
export RUST_LOG="${RUST_LOG:-info,helix_im=debug,helix_driver_native=info}"

HLOG="/tmp/helix_devloop_helix_$(date +%s).log"

echo "▶ [1/4] 构建 host-cli"
cargo build -p helix-host-cli --quiet || { echo "构建失败"; exit 1; }

echo "▶ [2/4] Go 日志基线 + 跑 helix ${DUR}s（cookieId=${HELIX_COOKIE_ID} companyId=${HELIX_COMPANY_ID:-<none>}）"
GOBASE=$(wc -l < "$GOLOG" 2>/dev/null || echo 0)
timeout "$DUR" ./target/debug/helix-host-cli > "$HLOG" 2>&1

echo ""
echo "════════ ① 代码层：helix 吐的 Effect / 握手 / emit ════════"
grep -iE "handshake|increment HTTP|DomainEvent emit|disconnected|hello" "$HLOG" | sed -E 's/\x1b\[[0-9;]*m//g' | sed -E 's/^[0-9T:.+-]+Z?\s*//' | head -15

echo ""
echo "════════ ② 日志层：Go RunIncrementWorker 链路（delta 自基线）════════"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "increment|RunIncrementWorker|parallel load|grpc|loadTeam|LoadIncrement|channelIds|publish|company|error" \
  | grep -oE '"caller":"[^"]*"|"msg":"[^"]*"|"error":"[^"]*"|"method":"[^"]*"' \
  | paste -sd '|' - 2>/dev/null | tr '|' '\n' | sed 's/^/  /' | head -30
# 兜底：若上面过滤太干，给原始关键行（截断）
echo "  --- 关键原始行 ---"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null \
  | grep -iE "parallel load|grpc service|RunIncrementWorker|LoadIncrement|increment_channel" \
  | cut -c1-200 | sed 's/^/  /' | head -10

echo ""
echo "════════ ③ 结果层：helix WS 收到的 increment 帧 ════════"
INC=$(grep -c "increment_channel" "$HLOG" 2>/dev/null); INC=${INC:-0}
grep -oE "\"action\":\"[a-z_]+\"" "$HLOG" 2>/dev/null | sort | uniq -c | sed 's/^/  /'
echo "  → increment_channel/_end 帧数: ${INC}"

echo ""
echo "════════ 闭环判定 ════════"
if [ "$INC" -gt 0 ]; then
  echo "  ✅ 收到 increment 帧 — push 链路通"
else
  echo "  ❌ 0 increment 帧 — 看 ② 末尾 error 定位 Go 侧失败点（grpc/parallel load/...）"
fi
echo "  helix 全日志: $HLOG"
echo "  Go 日志 delta: tail -n +$((GOBASE+1)) $GOLOG"
