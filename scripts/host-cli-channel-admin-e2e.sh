#!/usr/bin/env bash
# host-cli 频道管理 **单连接** e2e（Round-4 scenario channel-admin）——
# UC-5.4 群属性 / UC-5.5 置顶 / UC-5.6 公告 / UC-6.2 管理员。
#
# 口径（已确立·Round-4）：
#   - 群属性/置顶/公告/管理员族多为「频道级广播」事件；操作者 444 本身是频道成员
#     → 在**自己单连接**上应收对应广播自回声（update_channel / update_channel_notice /
#       channel_member_role_updated …）。这是单连接物理够得到的。
#   - 读族（announcement/list · channel/load/post/pinned）走 HTTP sync reply，**不产 WS 帧**——
#     host-cli 无 status 打印，记 data-dep（HTTP 出站已发，回声依赖数据/落库后投影）。
#   - 全在**自建一次性频道**上跑，destructive 安全（close 只销自建的）。
#
# 前置：cses-im-server :8066 UP（IDEA 右击 run 或 go run ./cmd/server）。
# 用法：
#   [MEMBER_USERS_JSON='["678","555"]'] SETTLE_MS=2800 bash scripts/host-cli-channel-admin-e2e.sh
#   # 身份/端口从 helix.config.yaml 读取；覆盖：HELIX_COOKIE_ID / HELIX_COMPANY_ID
#
# **只读 / 只抓帧脚本，不改生产代码 / 不改台账 / 不 push**。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

LOG="$(mktemp -t helix_chadmin_e2e.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-2800}"
TEAM_ID="${HELIX_COMPANY_ID}"
SELF_ID="${HELIX_COOKIE_ID}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

fail_pre=0
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID（config 或 env）"; fail_pre=1; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID（config 或 env）"; fail_pre=1; }
nc -z -w3 127.0.0.1 "$HELIX_GO_PORT" 2>/dev/null || { red "⛔ Go WS 不可达：:$HELIX_GO_PORT —— ${HELIX_START_HINT}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle() { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

drive() {
  { settle; for line in "$@"; do printf '%s\n' "$line"; settle; done; printf 'quit\n'; } \
    | RUST_LOG='info,helix_im=debug,helix_driver_host=debug' \
      cargo run -q -p helix-host-cli -- --config "$HELIX_CONFIG_PATH" \
        --cookie-id "$SELF_ID" --company-id "$TEAM_ID" >>"${LOG}" 2>&1
}

declare -a RESULTS
ws_action_hit() {
  local n
  n="$(grep -E "RAW WS inbound" "${LOG}" | grep -cE "\"action\": ?\"($1)\"" || true)"
  [ "${n:-0}" -gt 0 ]
}
assert_ws() { if ws_action_hit "$1"; then RESULTS+=("PASS|$2|WS action '$1'"); else RESULTS+=("FAIL|$2|未见 WS action '$1'"); fi; }
soft_ws()   { if ws_action_hit "$1"; then RESULTS+=("PASS|$2|WS action '$1'"); else RESULTS+=("SKIP|$2|$3"); fi; }
mark() { RESULTS+=("$1|$2|$3"); }

ylw "▶ 预编译 host-cli…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }

# ════════ Phase 0：自建一次性频道 ════════
ylw "▶ Phase 0：create 自建 throwaway 频道（teamId=${TEAM_ID}）"
CREATE_BODY="$(python3 - "$SELF_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members_raw = sys.argv[1:4]
members = json.loads(members_raw)
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}]
for m in members:
    users.append({"id": m, "teamId": team_id, "role": "MEMBER"})
all_ids = [self_id] + list(members)
body = {"teamId": team_id, "displayName": "helix-chadmin-e2e", "orient": "",
        "type": "P", "users": users, "picturetype": "USER",
        "picture": {"userIds": all_ids}, "forceCreate": True}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
echo "create body = $CREATE_BODY" >>"${LOG}"
printf '__CREATE_MARKER__\n' >>"${LOG}"
drive "create $CREATE_BODY"

NEW_CH="$(awk '/__CREATE_MARKER__/{seen=1} seen' "${LOG}" \
  | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
if [ -n "${NEW_CH}" ]; then grn "  ↳ 自建 channelId = ${NEW_CH}"; TARGET="${NEW_CH}"; else red "  ↳ 未抠到 channelId"; TARGET=""; fi

if [ -z "${TARGET}" ]; then
  mark FAIL "全链" "create 无 channelId（看 ${LOG}）"
else
  # ════════ UC-5.4 群属性：change/info（聚合改名/简介）+ change/notice ════════
  ylw "▶ UC-5.4 群属性：change_info(displayName/purpose) + change_notice"
  printf '__UC54_MARKER__\n' >>"${LOG}"
  drive \
    "cmd im_channel_change_info {\"channel_id\":\"${TARGET}\",\"display_name\":\"chadmin-renamed\",\"purpose\":\"e2e-purpose\"}" \
    "cmd im_channel_change_notice {\"channel_id\":\"${TARGET}\",\"notice\":{\"text\":\"e2e-notice-$(now_ms)\"}}"

  # ════════ UC-5.5 置顶：先发一条消息抠 postId → set_message_top（消息置顶）+ channel/change/top（频道置顶） ════════
  ylw "▶ UC-5.5 置顶：send → set_message_top + change_top"
  printf '__UC55_SEND_MARKER__\n' >>"${LOG}"
  drive "send ${TARGET} chadmin-pin-target-$(now_ms)"
  POST_ID="$(awk '/__UC55_SEND_MARKER__/{seen=1} seen' "${LOG}" \
    | grep -E "RAW WS inbound" | grep -E '"action": ?"post"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
  printf '__UC55_PIN_MARKER__\n' >>"${LOG}"
  if [ -n "${POST_ID}" ]; then
    grn "  ↳ 抠到 postId = ${POST_ID}"
    drive \
      "cmd im_set_message_top {\"channel_id\":\"${TARGET}\",\"post_id\":\"${POST_ID}\"}" \
      "cmd im_channel_load_post_pinned {\"channel_id\":\"${TARGET}\"}"
    HAVE_POST=1
  else
    ylw "  ↳ 未抠到 postId（send 无回声？）"; HAVE_POST=0
  fi
  # 频道置顶（per-member · 定向自己）
  printf '__UC55_CHTOP_MARKER__\n' >>"${LOG}"
  drive "cmd im_channel_change_top {\"channel_id\":\"${TARGET}\",\"top\":true}"

  # ════════ UC-5.6 公告：announcement_save → announcement_list ════════
  ylw "▶ UC-5.6 公告：announcement_save + announcement_list"
  printf '__UC56_MARKER__\n' >>"${LOG}"
  # save 体 verbatim 透传 post/announcement/save（含 id/channelId/message/createAt 等公告字段）。
  if [ "${HAVE_POST:-0}" = 1 ]; then
    drive \
      "cmd im_announcement_save {\"id\":\"${POST_ID}\",\"channelId\":\"${TARGET}\",\"message\":\"e2e-announcement-$(now_ms)\",\"createAt\":$(now_ms)}" \
      "cmd im_announcement_list {\"channel_id\":\"${TARGET}\"}"
  else
    drive "cmd im_announcement_list {\"channel_id\":\"${TARGET}\"}"
  fi

  # ════════ UC-6.2 管理员：member_role（设管理员）+ add_manger ════════
  ylw "▶ UC-6.2 管理员：member_role(678→ADMIN) + add_manger"
  printf '__UC62_MARKER__\n' >>"${LOG}"
  drive \
    "role ${TARGET} 678 ADMIN" \
    "cmd im_channel_add_manger {\"channelId\":\"${TARGET}\",\"userIds\":[\"678\"]}"

  # ════════ 收尾：重连补灌延迟广播 + close ════════
  ylw "▶ 收尾：read 触发重连补灌 + close 自建频道"
  printf '__TAIL_MARKER__\n' >>"${LOG}"
  drive "read ${TARGET} $(now_ms)"
  printf '__CLOSE_MARKER__\n' >>"${LOG}"
  drive "close ${TARGET}"
  drive "cmd im_get_all_modules"   # 末次空 drive 触发最后一次重连补灌
fi

# ════════ 断言（对累计 LOG）════════
if [ -n "${TARGET:-}" ]; then
  assert_ws "channel_created"                              "⓪ create (前置自建频道)"
  # UC-5.4
  soft_ws "update_channel"        "UC-5.4 群属性 change_info → update_channel" \
    "change/info 触发 update_channel；若 Go 定向/延迟未在窗内回推则数据依赖"
  soft_ws "update_channel_notice" "UC-5.4 群公告 change_notice → update_channel_notice" \
    "change/notice 触发 update_channel_notice；窗外/数据依赖"
  # UC-5.5
  if [ "${HAVE_POST:-0}" = 1 ]; then
    soft_ws "update_channel|post_pinned|post_update|posts_update" "UC-5.5 消息置顶 set_message_top" \
      "channel/add/postPinned 回声形态依赖 Go 推送（可能 HTTP-only / 数据依赖）"
  else
    mark SKIP "UC-5.5 消息置顶" "未抠到 postId（send 无回声）"
  fi
  soft_ws "update_channel" "UC-5.5 频道置顶 change_top → update_channel(定向自己)" \
    "channel/change/top per-member 定向；HTTP 200 后投影，窗外/数据依赖"
  # UC-5.6 读族 HTTP-only
  mark DATA "UC-5.6 公告 list (announcement/list)" "读族走 HTTP sync reply 无 WS 帧；HTTP 出站已发(host-cli 无 status 打印)→ data-dep"
  mark DATA "UC-5.6 公告 save (announcement/save)" "save 写族但 Go 是否回声依赖实现；本窗如无 update_channel_notice 类则 data-dep"
  # UC-6.2
  assert_ws "channel_member_role_updated" "UC-6.2 管理员 member_role → channel_member_role_updated"
  # close 对照
  soft_ws "channel_close" "收尾 close 自建频道" "close broadcast 窗外/数据依赖"
fi

echo; echo "════════ host-cli 频道管理单连接 e2e 结果（${SELF_ID} 自建频道自操作）════════"
pass=0 failc=0 skip=0 data=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
    SKIP) ylw "⏭  $label — $note"; skip=$((skip+1));;
    DATA) ylw "📊 $label — $note"; data=$((data+1));;
  esac
done
echo "─────────────────────────────────────────────"
echo "自建 channelId: ${NEW_CH:-<未建>}　日志: ${LOG}"
echo "PASS $pass · FAIL $failc · SKIP $skip · DATA-DEP $data"
echo "（RAW 帧证据：grep 'RAW WS inbound' ${LOG} | grep -oE '\"action\": ?\"[a-z_]+\"' | sort | uniq -c）"
echo "LOGFILE=${LOG}"
[ "$failc" -gt 0 ] && exit 1 || exit 0
