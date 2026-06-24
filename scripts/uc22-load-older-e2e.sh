#!/usr/bin/env bash
# UC-2.2 上拉历史 e2e —— 持久单连接，复用 host-cli-go-chain-e2e-persistent.sh 骨架。
#
# 链路：create throwaway 频道 → send N 条（建历史）→ 抠最新 post 的 id+createAt 作锚点
#   → `cmd im_load_older_context {channel_id, anchor_post_id, anchor_create_at, limit}`
#   → 观察 LOG 里 DomainEvent emit event=im:messages:older_loaded（older_context.rs:283）。
#
# 证据口径：
#   - older_loaded 是 Effect::Emit（非 WS inbound 帧），host-cli main.rs:270 打印
#     `DomainEvent emit event=im:messages:older_loaded`（tracing），故抠这一行作 verified 证据。
#   - send 自echo `post`（WS inbound）携 id/createAt → 锚点真源。
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
LOG="$(mktemp -t helix_uc22.XXXXXX.log)"
WAIT_TO="${WAIT_TO:-12}"; IDLE_HOLD_S=240
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
TEAM_ID="${HELIX_COMPANY_ID:-}"; SELF_ID="${HELIX_COOKIE_ID:-}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"
red(){ printf '\033[31m%s\033[0m\n' "$*"; }; grn(){ printf '\033[32m%s\033[0m\n' "$*"; }; ylw(){ printf '\033[33m%s\033[0m\n' "$*"; }
[ -z "$SELF_ID" ] && { red "缺 HELIX_COOKIE_ID"; exit 2; }
[ -z "$TEAM_ID" ] && { red "缺 HELIX_COMPANY_ID"; exit 2; }
nc -z -w3 127.0.0.1 8065 || { red "Go 不可达"; exit 2; }
now_ms(){ python3 -c 'import time;print(int(time.time()*1000))'; }
settle(){ python3 -c 'import time;time.sleep(1.2)'; }

TX_FIFO="$(mktemp -u -t helix_uc22_fifo.XXXXXX)"; CLI_PID=""; HOLDER_PID=""
cleanup(){ exec 9>&- 2>/dev/null||true; [ -n "$HOLDER_PID" ]&&kill "$HOLDER_PID" 2>/dev/null||true; [ -n "$CLI_PID" ]&&wait "$CLI_PID" 2>/dev/null||true; rm -f "$TX_FIFO" 2>/dev/null||true; }
trap cleanup EXIT INT TERM
mkfifo "$TX_FIFO"
ylw "▶ build host-cli"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ 起持久 host-cli"
HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API_BASE" \
  HELIX_COOKIE_ID="$SELF_ID" HELIX_COMPANY_ID="$TEAM_ID" \
  cargo run -q -p helix-host-cli <"$TX_FIFO" >>"$LOG" 2>&1 &
CLI_PID=$!; exec 9>"$TX_FIFO"; ( sleep "$IDLE_HOLD_S" )& HOLDER_PID=$!
feed(){ printf '%s\n' "$1" >&9; }
wait_for(){ local act="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | grep -E "RAW WS inbound" | grep -qE "\"action\": ?\"($act)\""; then return 0; fi
    python3 -c 'import time;time.sleep(0.2)'; t=$((t+1)); done; return 1; }

ylw "▶ 等 hello"; wait_for "hello" 15 && grn "  hello ✓" || red "  hello 未到"

# ── create throwaway 频道 ──
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json,sys
self_id,team_id,members=sys.argv[1],sys.argv[2],json.loads(sys.argv[3])
users=[{"id":self_id,"teamId":team_id,"role":"CREATOR"}]+[{"id":m,"teamId":team_id,"role":"MEMBER"} for m in members]
print(json.dumps({"teamId":team_id,"displayName":"helix-uc22","orient":"","type":"P","users":users,"picturetype":"USER","picture":{"userIds":[self_id]+list(members)},"forceCreate":True},ensure_ascii=False,separators=(",",":")))
PY
)"
printf '__CREATE__\n' >>"$LOG"; ylw "▶ create"; feed "create $CREATE_BODY"
wait_for "channel_created" "$WAIT_TO" "__CREATE__" || ylw "  create 回声未到"
NEW_CH="$(awk '/__CREATE__/{s=1}s' "$LOG"|grep "RAW WS inbound"|grep '"action": *"channel_created"'|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
[ -z "$NEW_CH" ] && { red "无 channelId，中止（LOG=$LOG）"; cleanup; trap - EXIT; exit 1; }
grn "  channelId=$NEW_CH"

# ── send 多条建历史，抠最后一条 post 的 id+createAt 作锚点 ──
for i in 1 2 3 4 5; do
  printf "__SEND_%s__\n" "$i" >>"$LOG"
  feed "send $NEW_CH uc22-hist-$i-$(now_ms)"
  wait_for "post" "$WAIT_TO" "__SEND_${i}__" || ylw "  send#$i 回声未到"
done
# 最后一条 post 帧（锚点）
ANCHOR_LINE="$(awk '/__SEND_5__/{s=1}s' "$LOG"|grep "RAW WS inbound"|grep '"action": *"post"'|tail -1)"
ANCHOR_ID="$(printf '%s' "$ANCHOR_LINE"|grep -oE '"id":"[a-z0-9]{26}"'|head -1|sed -E 's/.*"id":"([^"]+)".*/\1/')"
ANCHOR_AT="$(printf '%s' "$ANCHOR_LINE"|grep -oE '"createAt":[0-9]+'|head -1|sed -E 's/.*:([0-9]+)/\1/')"
grn "  anchor postId=$ANCHOR_ID createAt=$ANCHOR_AT"
[ -z "$ANCHOR_ID" ] && { red "无 anchor postId（LOG=$LOG）"; cleanup; trap - EXIT; exit 1; }
[ -z "$ANCHOR_AT" ] && ANCHOR_AT="$(now_ms)"

# ── 驱动 im_load_older_context ──
OLDER_BODY="$(python3 - "$NEW_CH" "$ANCHOR_ID" "$ANCHOR_AT" <<'PY'
import json,sys
print(json.dumps({"channel_id":sys.argv[1],"anchor_post_id":sys.argv[2],"anchor_create_at":int(sys.argv[3]),"limit":20},separators=(",",":")))
PY
)"
printf '__OLDER__\n' >>"$LOG"; ylw "▶ cmd im_load_older_context $OLDER_BODY"
feed "cmd im_load_older_context $OLDER_BODY"
# 等 older_loaded emit（tracing line，非 WS 帧）
t=0; HIT=0
while [ "$t" -lt "$((WAIT_TO*5))" ]; do
  if awk '/__OLDER__/{s=1}s' "$LOG"|grep -q "im:messages:older_loaded"; then HIT=1; break; fi
  python3 -c 'import time;time.sleep(0.2)'; t=$((t+1))
done
[ "$HIT" = 1 ] && grn "  older_loaded emit ✓" || red "  older_loaded emit 未观察到"

cleanup; trap - EXIT INT TERM
echo; echo "════════ UC-2.2 证据抠取 ════════"
echo "--- create channel_created 帧 ---"; awk '/__CREATE__/{s=1}s' "$LOG"|grep "RAW WS inbound"|grep '"action": *"channel_created"'|head -1|cut -c1-400
echo "--- 锚点 post 帧（最后一条 send 自echo）---"; printf '%s\n' "$ANCHOR_LINE"|cut -c1-400
echo "--- postContext 出站 HTTP（tracing）---"; awk '/__OLDER__/{s=1}s' "$LOG"|grep -iE "postContext|posts/postContext|http.*post" |head -3
echo "--- older_loaded emit 行 ---"; awk '/__OLDER__/{s=1}s' "$LOG"|grep "im:messages:older_loaded"|head -3
echo "--- DomainEvent emit 全量（OLDER 段）---"; awk '/__OLDER__/{s=1}s' "$LOG"|grep "DomainEvent emit"|head -10
echo "── LOG=$LOG"
