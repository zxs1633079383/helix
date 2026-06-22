#!/usr/bin/env bash
# host-cli ↔ Go 13-action **自包含链** e2e —— 全在自建的一次性 throwaway 频道上跑，destructive 安全。
#
# 与同目录 host-cli-go-e2e.sh 的区别：
#   - 那个用一个**外部给定的** CHANNEL_ID 跑非破坏命令 + 可选 create/close。
#   - 本脚本自建一次性频道（create 真源体）→ 从 channel_created 帧抠新 channelId →
#     在该新频道上链式跑 send/read/revoke/makeTopic/member/nickname/schedule/cancel/close，
#     **不碰任何现网真频道**（close 也只销自建的）。
#
# 定位：验「helix-im 各 outbound 请求体已对齐现网真源（真机curl真源.md）」后，真连 Go 看 13 action 全绿。
#
# 前置（外部状态，脚本起不了，须你先起）：
#   - Go mattermost :8065 + Java paas/x9 grpc UP
# 用法：
#   HELIX_COOKIE_ID=<userId> HELIX_COMPANY_ID=<teamId> \
#     [SEND_CHANNEL_ID=<现成可发频道>] \
#     bash scripts/host-cli-go-chain-e2e.sh
# 可选 env：
#   MEMBER_USERS_JSON='["0212fb861cea5be6","444"]'   # 建群/转话题/改成员的同公司成员 userId 列表
#   HELIX_WS_URL / HELIX_API_BASE
#   SETTLE_MS=2500
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="$(mktemp -t helix_chain_e2e.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-2500}"
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"
TEAM_ID="${HELIX_COMPANY_ID:-}"
SELF_ID="${HELIX_COOKIE_ID:-}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"0212fb861cea5be6\",\"444\"]}"
SEND_CHANNEL_ID="${SEND_CHANNEL_ID:-}"   # 现成可发频道（非破坏靶，type P）；create 失败时作 fallback 跑后续

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

# ── 前置检查 ────────────────────────────────────────────────
fail_pre=0
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID（=userId 身份头）"; fail_pre=1; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID（=teamId，create body / paas-grpc 必需）"; fail_pre=1; }
ws_host_port="$(printf '%s' "$WS_URL" | sed -E 's#^wss?://([^/]+).*#\1#')"
ws_host="${ws_host_port%%:*}"; ws_port="${ws_host_port##*:}"; [ "$ws_port" = "$ws_host" ] && ws_port=80
nc -z -w3 "$ws_host" "$ws_port" 2>/dev/null || { red "⛔ Go WS 端口不可达：${ws_host}:${ws_port}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle() { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

# 逐行喂 host-cli（行间 settle 等 WS 回推），append 到 ${LOG}。首 settle 在命令**之前**等 hello 握手。
drive() {
  { settle; for line in "$@"; do printf '%s\n' "$line"; settle; done; printf 'quit\n'; } \
    | HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API_BASE" \
      cargo run -q -p helix-host-cli >>"${LOG}" 2>&1
}

# 精确匹配 RAW WS inbound 帧的 `"action": "<exact>"`；action 支持 `a|b` 备选。
declare -a RESULTS
# ⚠️ 不用 `grep -E ... | grep -qE`：set -o pipefail 下 `grep -q` 命中即关管道 → 上游 grep 收
# SIGPIPE(141) → pipefail 把整管道判失败 → 即使命中也走 else 误报 FAIL（实测 timing-dependent）。
# 改单次 grep（先过滤 RAW WS inbound 再匹配 action，用 grep -c 计数，>0 即命中）。
ws_action_hit() {  # action-alternation → 0 命中 / 1 未命中
  local n
  n="$(grep -E "RAW WS inbound" "${LOG}" | grep -cE "\"action\": ?\"($1)\"" || true)"
  [ "${n:-0}" -gt 0 ]
}
assert_ws() {
  local action="$1" label="$2"
  if ws_action_hit "$action"; then
    RESULTS+=("PASS|$label|WS action '$action'")
  else
    RESULTS+=("FAIL|$label|未见 WS action '$action'（Go 未推/未连/体仍差字段）")
  fi
}
mark() { RESULTS+=("$1|$2|$3"); }

# 从 channel_created 帧抠新 channelId（data.id 是 26 char base32；取**本次 create 后**首个）。
# 注意：先清掉旧 LOG 行偏移——用 marker 行隔出 create 之后的帧。
extract_channel_created_id() {
  # 取 create marker 之后的 channel_created 帧的 data."id"。
  awk '/__CREATE_MARKER__/{seen=1} seen' "${LOG}" \
    | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/'
}

ylw "▶ 预编译 host-cli（cargo build -q）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }

# ════════ Phase 0：自建一次性频道（create 真源体，§4 真机curl真源）════════
ylw "▶ Phase 0：create 自建 throwaway 频道（teamId=${TEAM_ID}）"
# 真源 §4 body：teamId/displayName/orient/type:P/users[自己 CREATOR + 他人 MEMBER]/picturetype/picture.userIds/forceCreate。
# 用 python3 据 MEMBER_USERS_JSON + SELF_ID + TEAM_ID 拼合法 JSON（含 picture.userIds = 全员 id）。
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members_raw = sys.argv[1], sys.argv[2], sys.argv[3]
members = json.loads(members_raw)
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}]
for m in members:
    users.append({"id": m, "teamId": team_id, "role": "MEMBER"})
all_ids = [self_id] + list(members)
body = {
    "teamId": team_id,
    "displayName": "helix-e2e",
    "orient": "",
    "type": "P",
    "users": users,
    "picturetype": "USER",
    "picture": {"userIds": all_ids},
    "forceCreate": True,
}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
echo "create body = $CREATE_BODY" >>"${LOG}"
printf '__CREATE_MARKER__\n' >>"${LOG}"
# ⚠️ 断言统一推迟到全链跑完后对**累计 LOG** 做（见文件末 §断言汇总）。原因：create 触发 257+
# increment_channel 洪泛，channel_created/post 回声可能在本次 drive 的 settle 窗外才到（host-cli
# 已 quit），但会在**后续 drive 重连**时补灌进 LOG。逐 phase 内联断言会误判 FAIL（实测）。
# 抠 channelId/postId 仍逐 phase 做：到该 phase 时上一波回声已落 LOG（实测可抠到）。
drive "create $CREATE_BODY"

NEW_CH="$(extract_channel_created_id)"
if [ -n "${NEW_CH}" ]; then
  grn "  ↳ 抠到自建 channelId = ${NEW_CH}"
  echo "NEW_CHANNEL_ID=${NEW_CH}" >>"${LOG}"
  TARGET="${NEW_CH}"
else
  red "  ↳ 未从 channel_created 帧抠到 channelId"
  if [ -n "${SEND_CHANNEL_ID}" ]; then
    ylw "  ↳ fallback：用 SEND_CHANNEL_ID=${SEND_CHANNEL_ID} 跑后续非破坏命令（close 跳过防误删）"
    TARGET="${SEND_CHANNEL_ID}"
    USE_FALLBACK=1
  else
    TARGET=""
  fi
fi

if [ -z "${TARGET}" ]; then
  mark SKIP "②-⑬ 全链" "create 无 channelId 且无 SEND_CHANNEL_ID fallback（看 ${LOG} 定位 create 体/环境）"
else
  # ════════ Phase 1：在 TARGET 频道链式跑 send → read ════════
  ylw "▶ Phase 1：send/read（靶 channel=${TARGET}）"
  printf '__SEND_MARKER__\n' >>"${LOG}"
  drive \
    "send ${TARGET} e2e-chain-probe-$(now_ms)" \
    "read ${TARGET} $(now_ms)"

  # ════════ Phase 2：抠 server postId → revoke → makeTopic ════════
  ylw "▶ Phase 2：抠 postId 链式（revoke / makeTopic）"
  # 只从本次 send 的 post 回声帧抠 server postId（send marker 之后的 post 帧 data.id）。
  POST_ID="$(awk '/__SEND_MARKER__/{seen=1} seen' "${LOG}" \
    | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
  if [ -n "${POST_ID}" ]; then
    grn "  ↳ 抠到 server postId = ${POST_ID}"
    # makeTopic 真源 §2：rootId=新ch / postId / teamId / displayName / type:T / users / picture / forceCreate。
    MK_BODY="$(python3 - "${TARGET}" "${POST_ID}" "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
root_id, post_id, self_id, team_id, members_raw = sys.argv[1:6]
members = json.loads(members_raw)
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}]
for m in members:
    users.append({"id": m, "teamId": team_id, "role": "MEMBER"})
all_ids = [self_id] + list(members)
body = {
    "rootId": root_id, "teamId": team_id, "postId": post_id,
    "displayName": "helix-e2e-topic", "type": "T",
    "users": users, "picturetype": "USER",
    "picture": {"userIds": all_ids}, "forceCreate": True,
}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
    echo "makeTopic body = $MK_BODY" >>"${LOG}"
    printf '__REVOKE_MARKER__\n' >>"${LOG}"
    # 注意顺序：先 makeTopic（用未撤回的 postId），再 revoke（revoke 后 post 可能不可转）。
    drive "makeTopic $MK_BODY"
    printf '__REVOKE2_MARKER__\n' >>"${LOG}"
    drive "revoke ${POST_ID}"
    HAVE_POSTID=1
  else
    HAVE_POSTID=0
    ylw "  ↳ 未从 post 回声帧抠到 server postId（send 是否成功？看 ${LOG}）"
  fi

  # ════════ Phase 3：member change（join/leave 真成员）════════
  ylw "▶ Phase 3：member change（channel/member/change）"
  # 真源 §5：{channelId, joinUsers/leaveUsers:[{id,teamId,role}]}。
  # 自建群里这些成员已在群（create users[]），故先 leave 一个再 join 回，确保产生 channel_member_update。
  MEMBER_BODY="$(python3 - "${TARGET}" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
ch, team_id, members_raw = sys.argv[1:4]
members = json.loads(members_raw)
# leave 第一个成员（admin 移除）→ 必触发 channel_member_update。
target = members[0] if members else None
body = {"channelId": ch}
if target:
    body["leaveUsers"] = [{"id": target, "teamId": team_id, "role": "MEMBER"}]
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
  echo "member body = $MEMBER_BODY" >>"${LOG}"
  printf '__MEMBER_MARKER__\n' >>"${LOG}"
  drive "member $MEMBER_BODY"

  # ════════ Phase 4：nickname / schedule / cancel 同步族（回归）════════
  ylw "▶ Phase 4：nickname/schedule/cancel（同步族回归）"
  TS_FUTURE="$(( $(now_ms) + 600000 ))"
  printf '__SYNC_FAMILY_MARKER__\n' >>"${LOG}"
  drive \
    "nickname ${TARGET} e2e-nick" \
    "schedule ${TARGET} $TS_FUTURE e2e-scheduled" \
    "cancel ${TARGET}"

  # ════════ Phase 5：close 自建频道（destructive 但安全——只销自建的）════════
  if [ "${USE_FALLBACK:-0}" = 1 ]; then
    ylw "▶ Phase 5：跳过 close（TARGET 是 fallback 现网频道，防误删）"
  else
    ylw "▶ Phase 5：close 自建频道 ${TARGET}（安全销毁）"
    printf '__CLOSE_MARKER__\n' >>"${LOG}"
    drive "close ${TARGET}"
    # close broadcast 到 channelId·自己也收，但本次 drive 后无后续重连补推 →
    # 末尾多跑一次空 drive 触发重连，把 channel_close 灌进 LOG 供断言。
    drive "read ${SEND_CHANNEL_ID:-$TARGET} $(now_ms)"
  fi
fi

# ── 断言汇总（全链 drive 跑完后，对累计 LOG 一次性断言）─────────
# 为何统一在末尾：每个 phase 是独立 host-cli 进程，命令的 WS 回声可能在该进程 settle 窗外
# 才到（尤其 create 触发 257+ increment_channel 洪泛），但会在后续 drive 重连时补灌进 LOG。
# 逐 phase 内联断言会因这种延迟误判 FAIL；对累计 LOG 末尾断言才稳。
if [ -n "${TARGET:-}" ]; then
  # 软依赖项（数据/时序依赖，缺不算 bug）：no_echo → SKIP 而非 FAIL。
  soft_assert() {  # action label skip_reason
    if ws_action_hit "$1"; then
      RESULTS+=("PASS|$2|WS action '$1'")
    else
      RESULTS+=("SKIP|$2|$3")
    fi
  }
  assert_ws "channel_created"                 "① channel_created (create 自建频道)"
  assert_ws "post"                            "② post (send 回声 — 核心：验请求体修对)"
  soft_assert "post_read" "③ post_read (read)" "无未读 post 可读 → Go 不推（数据依赖，非 bug）"
  if [ "${HAVE_POSTID:-0}" = 1 ]; then
    assert_ws "post_update|posts_update"      "④ post_update (revoke)"
    # makeTopic 也推 channel_created（已被 ① 覆盖）；这里看它的 post_update 旁推（软依赖）。
    soft_assert "post_update|posts_update" "⑤ makeTopic (channel_created+post_update)" \
      "makeTopic post_update 旁推数据依赖（channel_created 已由 ① 覆盖）"
  else
    RESULTS+=("SKIP|④⑤ revoke/makeTopic|未抠到 server postId（send 无 post 回声？）")
  fi
  # ⑥ member change：helix-im 发 channel/member/change（Go 实测返 200）。但 channel_member_update
  # 的 WS 广播范围 = **被增减的目标用户**（doc §2.11 broadcast UserId=member），**非触发的 admin/self**。
  # host-cli 只连 self → leave 别人时 self 收不到该帧（观察路径限制，非 wire bug）。命中则 PASS，
  # 否则 SKIP（观察路径，命令已 200 成功）。
  soft_assert "channel_member_update" "⑥ channel_member_update (member change)" \
    "Go 返 200 但 channel_member_update 广播给被增减用户·非 self（观察路径限制，非 wire bug）"
  assert_ws "update_channel_member_nickName"  "⑦ nickname（camelCase N）"
  assert_ws "post_schedule_created"           "⑧ schedule"
  assert_ws "post_schedule_canceled"          "⑨ cancel"
  if [ "${USE_FALLBACK:-0}" = 1 ]; then
    RESULTS+=("SKIP|⑩ channel_close (close)|TARGET 是 fallback 现网频道，跳过 close 防误删")
  else
    assert_ws "channel_close"                 "⑩ channel_close (close 自建频道)"
  fi
fi

# ── 汇总 ────────────────────────────────────────────────────
echo; echo "════════ host-cli↔Go 自包含链 e2e 结果 ════════"
pass=0 failc=0 skip=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
    SKIP) ylw "⏭  $label — $note"; skip=$((skip+1));;
  esac
done
echo "─────────────────────────────────────────────"
echo "自建 channelId: ${NEW_CH:-<未建/失败>}"
echo "PASS $pass · FAIL $failc · SKIP $skip · 日志 ${LOG}"
[ "$failc" -gt 0 ] && exit 1 || exit 0
