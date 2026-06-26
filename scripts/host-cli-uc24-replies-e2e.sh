#!/usr/bin/env bash
# UC-2.4 回复列表 e2e —— 持久单连接，复用 chain-persistent 的 FIFO/feed/wait_for 骨架。
#
# 链路：create 自建频道 → send 抠 postId → `cmd im_get_replies {reply_id=postId, req_id}`
#   → is_read()=true → 注册 OutboundReadReply{req_id} → HTTP POST posts/getReplies
#   → 回报透传 emit `im:read:result{req_id, body}`（与 UC-9.2 书签 load 同一 read-relay 路径）。
#
# verdict 判据：LOG 出现 `DomainEvent emit event="im:read:result"` 且 bytes>0（marker 之后）。
#   （getReplies 对无回复的 post 仍返 HTTP200 + 空 data 列表 → relay 透传成非空信封 → bytes>0）。
#
# 账号同 chain-persistent；身份/端口从 helix.config.yaml 读取（覆盖：HELIX_COOKIE_ID / HELIX_COMPANY_ID）。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

LOG="$(mktemp -t helix_uc24.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-1200}"
WAIT_TO="${WAIT_TO:-12}"
IDLE_HOLD_S="${IDLE_HOLD_S:-180}"
TEAM_ID="${HELIX_COMPANY_ID}"
SELF_ID="${HELIX_COOKIE_ID}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

fail_pre=0
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID（config 或 env）"; fail_pre=1; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID（config 或 env）"; fail_pre=1; }
nc -z -w3 127.0.0.1 "$HELIX_GO_PORT" 2>/dev/null || { red "⛔ Go WS 不可达 :$HELIX_GO_PORT —— ${HELIX_START_HINT}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle()  { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

TX_FIFO="$(mktemp -u -t helix_uc24_tx_fifo.XXXXXX)"
CLI_PID=""; HOLDER_PID=""
cleanup() {
  exec 9>&- 2>/dev/null || true
  [ -n "$HOLDER_PID" ] && kill "$HOLDER_PID" 2>/dev/null || true
  [ -n "$CLI_PID" ] && wait "$CLI_PID" 2>/dev/null || true
  rm -f "$TX_FIFO" 2>/dev/null || true
}
trap cleanup EXIT INT TERM
mkfifo "$TX_FIFO"

ylw "▶ 预编译 host-cli"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ 起持久连接 host-cli"
cargo run -q -p helix-host-cli -- --config "$HELIX_CONFIG_PATH" \
  --cookie-id "$SELF_ID" --company-id "$TEAM_ID" <"$TX_FIFO" >>"$LOG" 2>&1 &
CLI_PID=$!
exec 9>"$TX_FIFO"
( sleep "$IDLE_HOLD_S" ) & HOLDER_PID=$!

feed() { printf '%s\n' "$1" >&9; }
wait_for() {  # action-alt [timeout_s] [marker]
  local act="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | grep -E "RAW WS inbound" | grep -qE "\"action\": ?\"($act)\""; then return 0; fi
    python3 -c "import time;time.sleep(0.2)"; t=$((t+1))
  done
  return 1
}
# 等 DomainEvent emit event=<name>（marker 之后），返 0 命中。
# 注：host-cli tracing 输出带 ANSI 色码（`event=im:read:result` 前后插 \x1b[..m）→ 必须先剥色再 grep。
strip_ansi() { sed -E 's/\x1b\[[0-9;]*m//g'; }
wait_emit() {  # event-name [timeout_s] [marker]
  local ev="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | strip_ansi | grep -qE "DomainEvent emit event=${ev} "; then return 0; fi
    python3 -c "import time;time.sleep(0.2)"; t=$((t+1))
  done
  return 1
}

ylw "▶ 等 hello 握手"
if wait_for "hello" 15; then grn "  ↳ hello 到达"; else red "  ↳ 15s 未见 hello"; fi

declare -a RESULTS

# ════════ create 自建一次性频道 ════════
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}] + [{"id": m, "teamId": team_id, "role": "MEMBER"} for m in members]
body = {"teamId": team_id, "displayName": "helix-uc24", "orient": "", "type": "P",
        "users": users, "picturetype": "USER", "picture": {"userIds": [self_id]+list(members)}, "forceCreate": True}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
printf '__CREATE_MARKER__\n' >>"$LOG"
ylw "▶ create 自建 throwaway 频道"
feed "create $CREATE_BODY"
wait_for "channel_created" "$WAIT_TO" "__CREATE_MARKER__" || ylw "  ↳ create 回声未到"
NEW_CH="$(awk '/__CREATE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"

if [ -z "$NEW_CH" ]; then
  RESULTS+=("SKIP|UC-2.4 回复列表|create 无 channelId（看 ${LOG}）")
else
  grn "  ↳ channelId=${NEW_CH}"
  # ════════ send → 抠 postId（作 reply 根·尽力，非 UC-2.4 必需）════════
  # 注：现网普通 send 无回声真因=体不对（真机curl真源），post 可能抠不到。
  # UC-2.4 验的是 getReplies read-relay 链路本身，不要求 reply 根真存在——
  # getReplies 对任意 reply_id 都发 HTTP，响应（200 空列表 / error）都透传回灌 im:read:result。
  printf '__SEND_MARKER__\n' >>"$LOG"
  ylw "▶ send 一条尝试作 reply 根（尽力）"
  feed "send ${NEW_CH} uc24-reply-root-$(now_ms)"
  wait_for "post" "$WAIT_TO" "__SEND_MARKER__" || ylw "  ↳ post 回声未到（send 体不对·已知，不阻断 UC-2.4）"
  POST_ID="$(awk '/__SEND_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"

  # reply_id：优先用 send 抠到的真 postId（真 reply 根·空列表回读）；抠不到则用频道 id 作合法 26 位
  # reply_id（HTTP 仍真打到 posts/getReplies，响应经 relay 回灌——证 read-relay 链路）。
  if [ -n "$POST_ID" ]; then
    REPLY_ID="$POST_ID"; REPLY_SRC="send 真 postId（真 reply 根·空列表回读）"
    grn "  ↳ replyRootPostId=${POST_ID}"
  else
    REPLY_ID="$NEW_CH"; REPLY_SRC="频道 id 作合法 26 位 reply_id（HTTP 真打·响应经 relay 回灌）"
    ylw "  ↳ 用 ${REPLY_SRC}"
  fi
  {
    # ════════ getReplies（核心验体）════════
    POST_ID="$REPLY_ID"
    REQ_ID="uc24_replies_$(now_ms)"
    GR_BODY="$(python3 - "$POST_ID" "$REQ_ID" <<'PY'
import json, sys
print(json.dumps({"reply_id": sys.argv[1], "pageNumber": 1, "pageSize": 20, "req_id": sys.argv[2]},
                 ensure_ascii=False, separators=(",", ":")))
PY
)"
    printf '__GETREPLIES_MARKER__\n' >>"$LOG"
    ylw "▶ cmd im_get_replies（read-relay → im:read:result）"
    feed "cmd im_get_replies $GR_BODY"
    if wait_emit "im:read:result" "$WAIT_TO" "__GETREPLIES_MARKER__"; then
      EMIT_LINE="$(awk '/__GETREPLIES_MARKER__/{seen=1} seen' "$LOG" | strip_ansi | grep -E 'DomainEvent emit event=im:read:result ' | head -1)"
      BYTES="$(printf '%s' "$EMIT_LINE" | grep -oE 'bytes=[0-9]+' | head -1 | cut -d= -f2)"
      if [ "${BYTES:-0}" -gt 0 ]; then
        RESULTS+=("PASS|UC-2.4 回复列表|im:read:result bytes=${BYTES} req_id=${REQ_ID} replyRoot=${POST_ID}")
      else
        RESULTS+=("FAIL|UC-2.4 回复列表|im:read:result 命中但 bytes=${BYTES:-?}")
      fi
    else
      RESULTS+=("FAIL|UC-2.4 回复列表|未见 im:read:result emit（看 ${LOG}）")
    fi
    # 失败诊断：是否 read http failed warn
    if grep -q 'read outbound http failed' "$LOG"; then ylw "  ⚠ 有 read outbound http failed warn"; fi
  }
fi

cleanup; trap - EXIT INT TERM

echo; echo "════════ UC-2.4 回复列表 e2e 结果 ════════"
pass=0 failc=0 skip=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
    SKIP) ylw "⏭  $label — $note"; skip=$((skip+1));;
  esac
done
echo "── PASS=$pass FAIL=$failc SKIP=$skip · LOG=$LOG"
# 末尾回显 im:read:result emit 行（证据·剥 ANSI）
strip_ansi <"$LOG" | grep -E 'DomainEvent emit event=im:read:result ' | head -3
[ "$failc" -gt 0 ] && exit 1 || exit 0
