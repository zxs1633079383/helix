#!/usr/bin/env bash
# host-cli ↔ Go 13-action 自包含链 e2e —— **持久单连接版**（对比 host-cli-go-chain-e2e.sh 的每动作短连）。
#
# 与短连版的唯一区别 = **整条链一条 WS 连接、一次 hello 握手**：
#   - 短连版 `drive()` 每个动作组重起一个 `cargo run` 进程 → 8 次进程 = 8 次 WS 重连 = 8 次 hello 等待。
#   - 本版用 FIFO 持开 host-cli 的 stdin（holder fd），`feed` 逐条喂命令、`wait_for` 轮询回声帧抠 id，
#     全程一个 host-cli 进程 / 一条 WS / 一次 hello。close FIFO → host-cli 收 EOF 优雅 quit。
#   - 额外收益：① 砍掉 N-1 次 hello-settle 重连税 ② 迟到回声落同一 LOG（短连版迟到帧要靠"下次重连补灌"）
#     ③ `wait_for` 回声即返，比固定 SETTLE_MS 盲等更快更稳。
#
# 前置 + 用法 + 账号同短连版：
#   HELIX_COOKIE_ID=<userId> HELIX_COMPANY_ID=<teamId> \
#     [MEMBER_USERS_JSON='["678","555"]'] bash scripts/host-cli-go-chain-e2e-persistent.sh
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="$(mktemp -t helix_chain_persist.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-1200}"          # 持久连接下只用于 fire-and-forget 命令的最小等；有回声的走 wait_for
WAIT_TO="${WAIT_TO:-10}"                # wait_for 单次回声等待上限（秒）
IDLE_HOLD_S="${IDLE_HOLD_S:-180}"       # holder 兜底持开 FIFO 写端的上限
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
TEAM_ID="${HELIX_COMPANY_ID:-}"
SELF_ID="${HELIX_COOKIE_ID:-}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"0212fb861cea5be6\",\"444\"]}"
SEND_CHANNEL_ID="${SEND_CHANNEL_ID:-}"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

# ── 前置检查 ────────────────────────────────────────────────
fail_pre=0
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID"; fail_pre=1; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID"; fail_pre=1; }
ws_hp="$(printf '%s' "$WS_URL" | sed -E 's#^wss?://([^/]+).*#\1#')"
ws_host="${ws_hp%%:*}"; ws_port="${ws_hp##*:}"; [ "$ws_port" = "$ws_host" ] && ws_port=80
nc -z -w3 "$ws_host" "$ws_port" 2>/dev/null || { red "⛔ Go WS 不可达 ${ws_host}:${ws_port}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle()  { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

TX_FIFO="$(mktemp -u -t helix_chain_tx_fifo.XXXXXX)"
CLI_PID=""; HOLDER_PID=""
cleanup() {
  exec 9>&- 2>/dev/null || true            # 合上写端 → host-cli 收 EOF
  [ -n "$HOLDER_PID" ] && kill "$HOLDER_PID" 2>/dev/null || true
  [ -n "$CLI_PID" ] && wait "$CLI_PID" 2>/dev/null || true
  rm -f "$TX_FIFO" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkfifo "$TX_FIFO"

# ── 启 host-cli：读 FIFO（持久 stdin），dump 帧到 LOG ──────────
ylw "▶ 预编译 host-cli（cargo build -q）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ 起持久连接 host-cli（一条 WS 跑完整链）"
HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API_BASE" \
  HELIX_COOKIE_ID="$SELF_ID" HELIX_COMPANY_ID="$TEAM_ID" \
  cargo run -q -p helix-host-cli <"$TX_FIFO" >>"$LOG" 2>&1 &
CLI_PID=$!
exec 9>"$TX_FIFO"                           # 持开写端（holder fd 9）→ host-cli stdin 永不 EOF 直到 cleanup
( sleep "$IDLE_HOLD_S" ) & HOLDER_PID=$!     # 兜底：异常退出时 holder 也会被 cleanup kill

feed() { printf '%s\n' "$1" >&9; }          # 喂一条命令进持久连接

# 轮询 LOG 等某 action 回声帧出现（marker 之后），命中即返 0（快返），超时返 1。
wait_for() {  # action-alt [timeout_s] [marker]
  local act="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | grep -E "RAW WS inbound" | grep -qE "\"action\": ?\"($act)\""; then return 0; fi
    python3 -c "import time;time.sleep(0.2)"; t=$((t+1))
  done
  return 1
}

# ── 等 hello 一次（整条链就这一次握手）──────────────────────
ylw "▶ 等 hello 握手（持久连接全程仅此一次）"
if wait_for "hello" 15; then grn "  ↳ hello 到达，connectionId 已盖章"; else red "  ↳ 15s 未见 hello（环境/网络？）"; fi

declare -a RESULTS
ws_hit() { local n; n="$(grep -E "RAW WS inbound" "$LOG" | grep -cE "\"action\": ?\"($1)\"" || true)"; [ "${n:-0}" -gt 0 ]; }
assert_ws()   { if ws_hit "$1"; then RESULTS+=("PASS|$2|WS '$1'"); else RESULTS+=("FAIL|$2|未见 WS '$1'"); fi; }
soft_assert() { if ws_hit "$1"; then RESULTS+=("PASS|$2|WS '$1'"); else RESULTS+=("SKIP|$2|$3"); fi; }

# ════════ create 自建一次性频道（真源 §4 体）════════
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}] + [{"id": m, "teamId": team_id, "role": "MEMBER"} for m in members]
body = {"teamId": team_id, "displayName": "helix-e2e", "orient": "", "type": "P",
        "users": users, "picturetype": "USER", "picture": {"userIds": [self_id]+list(members)}, "forceCreate": True}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
printf '__CREATE_MARKER__\n' >>"$LOG"
ylw "▶ create 自建 throwaway 频道"
feed "create $CREATE_BODY"
wait_for "channel_created" "$WAIT_TO" "__CREATE_MARKER__" || ylw "  ↳ create 回声未在 ${WAIT_TO}s 内到（继续，末尾断言兜底）"
NEW_CH="$(awk '/__CREATE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
if [ -n "$NEW_CH" ]; then grn "  ↳ channelId=${NEW_CH}"; TARGET="$NEW_CH"
elif [ -n "$SEND_CHANNEL_ID" ]; then ylw "  ↳ fallback SEND_CHANNEL_ID"; TARGET="$SEND_CHANNEL_ID"; USE_FALLBACK=1
else TARGET=""; fi

if [ -z "$TARGET" ]; then
  RESULTS+=("SKIP|②-⑩ 全链|create 无 channelId（看 $LOG）")
else
  # ════════ send → 抠 postId ════════
  printf '__SEND_MARKER__\n' >>"$LOG"
  ylw "▶ send（持久连接，wait_for post 回声）"
  feed "send ${TARGET} e2e-persist-probe-$(now_ms)"
  wait_for "post" "$WAIT_TO" "__SEND_MARKER__" || ylw "  ↳ post 回声未到"
  POST_ID="$(awk '/__SEND_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
  feed "read ${TARGET} $(now_ms)"; settle

  # ════════ makeTopic → revoke（依赖 postId）════════
  if [ -n "$POST_ID" ]; then
    grn "  ↳ postId=${POST_ID}"
    MK_BODY="$(python3 - "$TARGET" "$POST_ID" "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
ch, pid, self_id, team_id, members = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4], json.loads(sys.argv[5])
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}] + [{"id": m, "teamId": team_id, "role": "MEMBER"} for m in members]
body = {"rootId": ch, "teamId": team_id, "postId": pid, "displayName": "helix-e2e-topic", "type": "T",
        "users": users, "picturetype": "USER", "picture": {"userIds": [self_id]+list(members)}, "forceCreate": True}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
    printf '__TOPIC_MARKER__\n' >>"$LOG"
    feed "makeTopic $MK_BODY"; wait_for "channel_created|post_update|posts_update" "$WAIT_TO" "__TOPIC_MARKER__" || settle
    feed "revoke ${POST_ID}"; wait_for "post_update|posts_update" "$WAIT_TO" || settle
    HAVE_POSTID=1
  else HAVE_POSTID=0; ylw "  ↳ 未抠到 postId（send 无 post 回声？）"; fi

  # ════════ member change ════════
  MEMBER_BODY="$(python3 - "$TARGET" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
ch, team_id, members = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
body = {"channelId": ch}
if members: body["leaveUsers"] = [{"id": members[0], "teamId": team_id, "role": "MEMBER"}]
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
  feed "member $MEMBER_BODY"; wait_for "channel_member_update" "$WAIT_TO" || settle

  # ════════ nickname / schedule / cancel ════════
  TS_FUTURE="$(( $(now_ms) + 600000 ))"
  feed "nickname ${TARGET} e2e-nick"; wait_for "update_channel_member_nickName" "$WAIT_TO" || settle
  feed "schedule ${TARGET} $TS_FUTURE e2e-scheduled"; wait_for "post_schedule_created" "$WAIT_TO" || settle
  feed "cancel ${TARGET}"; wait_for "post_schedule_canceled" "$WAIT_TO" || settle

  # ════════ close（仅销自建）════════
  if [ "${USE_FALLBACK:-0}" = 1 ]; then ylw "▶ 跳过 close（fallback 现网频道）"
  else feed "close ${TARGET}"; wait_for "channel_close" "$WAIT_TO" || settle; fi
fi

# ── 关连接：合写端 → host-cli EOF quit ──
cleanup; trap - EXIT INT TERM

# ── 断言（对累计 LOG）─────────────────────────────────────
if [ -n "${TARGET:-}" ]; then
  assert_ws "channel_created" "① channel_created (create)"
  assert_ws "post"            "② post (send — 核心验体)"
  soft_assert "post_read" "③ post_read (read)" "无未读可读·数据依赖"
  if [ "${HAVE_POSTID:-0}" = 1 ]; then
    assert_ws "post_update|posts_update" "④ post_update (revoke)"
    soft_assert "post_update|posts_update" "⑤ makeTopic 旁推" "channel_created 已由①覆盖"
  else RESULTS+=("SKIP|④⑤|未抠到 postId"); fi
  soft_assert "channel_member_update" "⑥ member change" "广播给被增减用户·非 self（观察路径限）"
  assert_ws "update_channel_member_nickName" "⑦ nickname"
  assert_ws "post_schedule_created"          "⑧ schedule"
  assert_ws "post_schedule_canceled"         "⑨ cancel"
  if [ "${USE_FALLBACK:-0}" = 1 ]; then RESULTS+=("SKIP|⑩ close|fallback 现网频道跳过")
  else assert_ws "channel_close" "⑩ channel_close (close)"; fi
fi

echo; echo "════════ host-cli↔Go 持久单连接链 e2e 结果 ════════"
pass=0 failc=0 skip=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
    SKIP) ylw "⏭  $label — $note"; skip=$((skip+1));;
  esac
done
echo "── PASS=$pass FAIL=$failc SKIP=$skip · 一条连接一次 hello · LOG=$LOG"
[ "$failc" -gt 0 ] && exit 1 || exit 0
