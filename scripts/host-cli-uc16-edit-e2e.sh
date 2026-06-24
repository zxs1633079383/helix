#!/usr/bin/env bash
# UC-1.6「编辑消息」真相 e2e —— 持久单连接。
#
# 调查结论（待本脚本 wire 实证）：现网/真 Go **无消息正文编辑端点**。posts/create body 的 `id`
# 字段非空 ≠ 编辑——Go `CreateCsesPost → PreSave`（post.go:187）只在 `ID==""` 时铸新 id，非空则**保留**
# 调用方 id，但**没有任何 update 分支**（无 posts/update / posts/edit 路由）。故「id 非空 → Go 当 update」
# 是源码误读。本脚本：① send 普通消息抠 postId（确认 `post` 回声）② drive `cmd im_send_message` 复用该
# postId 作 body.id（模拟「编辑」）③ 观察真回声 action —— 若仍 `post`（新建）或零回声而非 `post_update`，
# 证实「编辑 = 不存在的能力 / 至多新建」。
#
#   HELIX_COOKIE_ID=<userId> HELIX_COMPANY_ID=<teamId> \
#     [MEMBER_USERS_JSON='["678","555"]'] bash scripts/host-cli-uc16-edit-e2e.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
LOG="$(mktemp -t helix_uc16.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-1500}"; WAIT_TO="${WAIT_TO:-12}"; IDLE_HOLD_S="${IDLE_HOLD_S:-180}"
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
TEAM_ID="${HELIX_COMPANY_ID:-}"; SELF_ID="${HELIX_COOKIE_ID:-}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"
red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID"; exit 2; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID"; exit 2; }
ws_hp="$(printf '%s' "$WS_URL" | sed -E 's#^wss?://([^/]+).*#\1#')"; ws_host="${ws_hp%%:*}"; ws_port="${ws_hp##*:}"
nc -z -w3 "$ws_host" "$ws_port" 2>/dev/null || { red "⛔ Go WS 不可达 ${ws_host}:${ws_port}"; exit 2; }
now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle()  { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }
TX_FIFO="$(mktemp -u -t helix_uc16_fifo.XXXXXX)"; CLI_PID=""; HOLDER_PID=""
cleanup() { exec 9>&- 2>/dev/null || true; [ -n "$HOLDER_PID" ] && kill "$HOLDER_PID" 2>/dev/null || true
  [ -n "$CLI_PID" ] && wait "$CLI_PID" 2>/dev/null || true; rm -f "$TX_FIFO" 2>/dev/null || true; }
trap cleanup EXIT INT TERM
mkfifo "$TX_FIFO"
ylw "▶ 预编译 host-cli"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API_BASE" \
  HELIX_COOKIE_ID="$SELF_ID" HELIX_COMPANY_ID="$TEAM_ID" \
  cargo run -q -p helix-host-cli <"$TX_FIFO" >>"$LOG" 2>&1 &
CLI_PID=$!; exec 9>"$TX_FIFO"; ( sleep "$IDLE_HOLD_S" ) & HOLDER_PID=$!
feed() { printf '%s\n' "$1" >&9; }
wait_for() { local act="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | grep -E "RAW WS inbound" | grep -qE "\"action\": ?\"($act)\""; then return 0; fi
    python3 -c "import time;time.sleep(0.2)"; t=$((t+1)); done; return 1; }

ylw "▶ 等 hello"; wait_for "hello" 15 && grn "  ↳ hello ✓" || red "  ↳ 15s 无 hello"

# ① create throwaway channel
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}] + [{"id": m, "teamId": team_id, "role": "MEMBER"} for m in members]
print(json.dumps({"teamId": team_id, "displayName": "helix-uc16", "orient": "", "type": "P",
  "users": users, "picturetype": "USER", "picture": {"userIds": [self_id]+list(members)}, "forceCreate": True}, ensure_ascii=False, separators=(",", ":")))
PY
)"
printf '__CREATE_MARKER__\n' >>"$LOG"; ylw "▶ create 频道"; feed "create $CREATE_BODY"
wait_for "channel_created" "$WAIT_TO" "__CREATE_MARKER__" || ylw "  ↳ create 回声未到"
TARGET="$(awk '/__CREATE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
[ -z "$TARGET" ] && { red "⛔ 无 channelId，看 $LOG"; exit 1; }
grn "  ↳ channelId=$TARGET"

# ② send 普通消息 → 抠 postId + 确认 `post` 回声
ORIG_TEXT="uc16-orig-$(now_ms)"
printf '__SEND_MARKER__\n' >>"$LOG"; ylw "▶ send 原始消息: $ORIG_TEXT"
feed "send ${TARGET} ${ORIG_TEXT}"
wait_for "post" "$WAIT_TO" "__SEND_MARKER__" && grn "  ↳ 原消息 'post' 回声 ✓" || red "  ↳ 原消息 post 回声未到"
POST_ID="$(awk '/__SEND_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
[ -z "$POST_ID" ] && { red "⛔ 抠不到 postId，看 $LOG"; exit 1; }
grn "  ↳ postId=$POST_ID"

# ③ 「编辑」尝试：drive cmd im_send_message，body.id 复用 POST_ID（模拟编辑同一条）
EDIT_TEXT="uc16-edited-$(now_ms)"
EDIT_CMD="$(python3 - "$TARGET" "$POST_ID" "$EDIT_TEXT" <<'PY'
import json, sys
ch, pid, text = sys.argv[1], sys.argv[2], sys.argv[3]
print(json.dumps({"channel_id": ch, "temporary_id": "hostcli_uc16_edit", "text": text, "id": pid}, ensure_ascii=False, separators=(",", ":")))
PY
)"
printf '__EDIT_MARKER__\n' >>"$LOG"; ylw "▶ 「编辑」尝试: cmd im_send_message id=$POST_ID text=$EDIT_TEXT"
feed "cmd im_send_message $EDIT_CMD"
# 等任意回声：post（新建/覆盖）/ post_update（真编辑）/ posts_update —— 12s
if wait_for "post_update|posts_update" "$WAIT_TO" "__EDIT_MARKER__"; then
  EDIT_ACTION="post_update"
elif wait_for "post" 3 "__EDIT_MARKER__"; then
  EDIT_ACTION="post"
else
  EDIT_ACTION="(零回声)"
fi
settle

echo "════════════════════════════════════════════════"
echo "UC-1.6 编辑真相 e2e 结果："
echo "  原消息 postId   = $POST_ID  (action=post ✓)"
echo "  「编辑」回声 action = $EDIT_ACTION"
echo "  ── RAW 编辑帧（__EDIT_MARKER__ 后）──"
awk '/__EDIT_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | head -8
echo "  LOG=$LOG"
case "$EDIT_ACTION" in
  post_update) grn "→ 真 Go 把 id 非空当 update（UC-1.6 = wire-bug 可修向）" ;;
  post)        red "→ 真 Go 仍当**新建**（id 非空≠编辑；编辑端点不存在 → UC-1.6 = not-a-capability）" ;;
  *)           red "→ 零回声（id 非空 posts/create 无回声；编辑能力不存在 → UC-1.6 = not-a-capability/data-dep）" ;;
esac
