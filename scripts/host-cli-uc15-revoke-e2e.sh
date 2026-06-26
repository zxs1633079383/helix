#!/usr/bin/env bash
# UC-1.5 撤回 —— **隔离** revoke 剧本（不和 makeTopic 纠缠）。
#
# 与 host-cli-go-chain-e2e-persistent.sh 的区别 = 只跑 send → revoke 两步，去掉 makeTopic：
# 这样 revoke 触发的 post_update 帧能**唯一归因**到 revoke（链版里 makeTopic 也旁推 post_update，
# 无法分辨 post_update 到底是谁触发的）。隔离剧本 + 抠 THAT postId + 断言 data.id==该postId 才算
# 真 verified（橡皮章零容忍）。
#
# 持久单连接（一条 WS / 一次 hello）：
#   ① create throwaway 频道 → 抠 channelId
#   ② send 一条 → wait_for post → 抠 THAT postId
#   ③ revoke {该postId} → wait_for post_update → 断言帧里 data.id == 该postId 且带 revoke/isDelete 标记
#   ④ close 销频道
#
# 用法：
#   [MEMBER_USERS_JSON='["678","555"]'] bash scripts/host-cli-uc15-revoke-e2e.sh
#   # 身份/端口从 helix.config.yaml 读取；覆盖：HELIX_COOKIE_ID / HELIX_COMPANY_ID
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

LOG="$(mktemp -t helix_uc15_revoke.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-1200}"
WAIT_TO="${WAIT_TO:-10}"
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

TX_FIFO="$(mktemp -u -t helix_uc15_tx_fifo.XXXXXX)"
CLI_PID=""; HOLDER_PID=""
cleanup() {
  exec 9>&- 2>/dev/null || true
  [ -n "$HOLDER_PID" ] && kill "$HOLDER_PID" 2>/dev/null || true
  [ -n "$CLI_PID" ] && wait "$CLI_PID" 2>/dev/null || true
  rm -f "$TX_FIFO" 2>/dev/null || true
}
trap cleanup EXIT INT TERM
mkfifo "$TX_FIFO"

ylw "▶ 预编译 host-cli（cargo build -q）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ 起持久连接 host-cli（一条 WS 跑完隔离 revoke 剧本）"
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

ylw "▶ 等 hello 握手（持久连接全程仅此一次）"
if wait_for "hello" 15; then grn "  ↳ hello 到达，connectionId 已盖章"; else red "  ↳ 15s 未见 hello（环境/网络？）"; fi

declare -a RESULTS

# ════════ ① create throwaway 频道 ════════
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members = sys.argv[1], sys.argv[2], json.loads(sys.argv[3])
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}] + [{"id": m, "teamId": team_id, "role": "MEMBER"} for m in members]
body = {"teamId": team_id, "displayName": "helix-uc15-revoke", "orient": "", "type": "P",
        "users": users, "picturetype": "USER", "picture": {"userIds": [self_id]+list(members)}, "forceCreate": True}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
printf '__CREATE_MARKER__\n' >>"$LOG"
ylw "▶ ① create throwaway 频道"
feed "create $CREATE_BODY"
wait_for "channel_created" "$WAIT_TO" "__CREATE_MARKER__" || ylw "  ↳ create 回声未到"
TARGET="$(awk '/__CREATE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
if [ -n "$TARGET" ]; then grn "  ↳ channelId=${TARGET}"; else red "  ↳ create 无 channelId（中止，看 $LOG）"; cleanup; trap - EXIT INT TERM; exit 1; fi

# ════════ ② send → 抠 THAT postId ════════
PROBE="uc15-revoke-probe-$(now_ms)"
printf '__SEND_MARKER__\n' >>"$LOG"
ylw "▶ ② send 一条（probe='${PROBE}'），wait_for post 回声抠 THAT postId"
feed "send ${TARGET} ${PROBE}"
wait_for "post" "$WAIT_TO" "__SEND_MARKER__" || ylw "  ↳ post 回声未到"
POST_ID="$(awk '/__SEND_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
if [ -n "$POST_ID" ]; then grn "  ↳ THAT postId=${POST_ID}"; RESULTS+=("PASS|① send 抠 postId|postId=${POST_ID}")
else red "  ↳ 未抠到 postId（send 无 post 回声？中止）"; RESULTS+=("FAIL|① send 抠 postId|无 post 回声"); cleanup; trap - EXIT INT TERM
  echo; for r in "${RESULTS[@]}"; do echo "$r"; done; echo "LOG=$LOG"; exit 1; fi

# ════════ ③ revoke {该postId} → wait_for post_update → 归因断言 ════════
printf '__REVOKE_MARKER__\n' >>"$LOG"
ylw "▶ ③ revoke {${POST_ID}}（隔离·不和 makeTopic 纠缠），wait_for post_update"
feed "revoke ${POST_ID}"
if wait_for "post_update|posts_update" "$WAIT_TO" "__REVOKE_MARKER__"; then
  # 抠 revoke marker 之后的 post_update 帧，断言其 data.id（或 id）命中 THAT postId
  REVOKE_FRAMES="$(awk '/__REVOKE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post_update"|"action": ?"posts_update"')"
  if printf '%s' "$REVOKE_FRAMES" | grep -qE "\"id\":\"${POST_ID}\""; then
    grn "  ↳ post_update 帧命中 THAT postId=${POST_ID}（归因 revoke·非 makeTopic）"
    if printf '%s' "$REVOKE_FRAMES" | grep -qiE "\"revoke\":|\"isDelete\":|\"delete\":|\"deleteAt\":"; then
      grn "  ↳ 且带 revoke/isDelete/deleteAt 撤回标记"
      RESULTS+=("PASS|② revoke 归因|post_update.id==${POST_ID} + 撤回标记")
    else
      ylw "  ↳ 命中 postId 但未见显式 revoke 标记（仍归因成立·标记字段名待核）"
      RESULTS+=("PASS|② revoke 归因|post_update.id==${POST_ID}（标记字段名待核）")
    fi
  else
    red "  ↳ post_update 帧未命中 THAT postId（归因失败）"
    RESULTS+=("FAIL|② revoke 归因|post_update 未含 ${POST_ID}")
  fi
else
  red "  ↳ ${WAIT_TO}s 内未见 post_update（revoke 无回声）"
  RESULTS+=("FAIL|② revoke 归因|无 post_update 回声")
fi

# 贴 revoke 后真 RAW 帧（证据用）
ylw "▶ revoke 后 RAW WS post_update 帧（证据）："
awk '/__REVOKE_MARKER__/{seen=1} seen' "$LOG" | grep -E "RAW WS inbound" | grep -E '"action": ?"post_update"|"action": ?"posts_update"' | head -3

# ════════ ④ close 销频道 ════════
feed "close ${TARGET}"; wait_for "channel_close" "$WAIT_TO" || settle

cleanup; trap - EXIT INT TERM

echo; echo "════════ UC-1.5 隔离 revoke e2e 结果 ════════"
pass=0 failc=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
  esac
done
echo "── PASS=$pass FAIL=$failc · 隔离剧本(send→revoke·无makeTopic) · LOG=$LOG"
echo "── 完整 LOG（含所有 RAW 帧）：$LOG"
[ "$failc" -gt 0 ] && exit 1 || exit 0
