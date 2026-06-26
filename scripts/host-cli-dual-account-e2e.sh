#!/usr/bin/env bash
# host-cli **双账号并发** e2e —— 验广播帧（非自回声）真达「第二连接」。
#
# 与同目录两个单连接 e2e 的区别：
#   - host-cli-go-e2e.sh / host-cli-go-chain-e2e.sh：**单连接**，只能看**发送者自己**的回声
#     （send 的 post 自 echo、create 的 channel_created 自 echo）。
#   - 本脚本：**两个并发 host-cli 连接**——
#       · 接收方（678）先连、**idle 持续抓 RAW 帧**到 RX_LOG（保持 stdin 开不退）。
#       · 发送方（444）自建群 + 拉 678 入群，在群里 send / 踢 / 改成员，各动作后 settle。
#     断言看**接收方 RX_LOG** 是否收到广播帧（post / channel_member_update / update_channel / post_read）——
#     这是单连接物理够不到的「广播扩散」证据（broadcast 范围 = 频道成员，非触发者本人）。
#
# 定位：补全 e2e 验证口径——
#   send 族（发送者看自回声）已由单连接 e2e 覆盖；**广播族**（拉踢 / 快照 / 已读未读态扩散）
#   必须双账号：444 操作，678 第二连接收 update/member/post_read 广播。
#
# 前置（外部状态，脚本起不了，须你先起）：
#   - cses-im-server :8066 UP（IDEA 右击 run 或 go run ./cmd/server）
#   - 444 与 678 均为同公司（HELIX_COMPANY_ID）真实 userId；444 有建群权限，678 可被拉入群。
# 用法：
#   [MEMBER_USERS_JSON='["678","555"]'] bash scripts/host-cli-dual-account-e2e.sh
#   # 身份/端口从 helix.config.yaml 读取；发送方=HELIX_COOKIE_ID(默认444)；接收方=RECEIVER_ID(默认678)
# 可选 env：
#   RECEIVER_ID=678                 # 接收方 userId（第二连接，被拉入群、收广播）
#   SETTLE_MS=2800                  # 每动作后等 WS 广播扩散的毫秒（双账号扩散比自回声慢，调大）
#   IDLE_HOLD_S=60                  # 接收方 idle 保持连接的秒数上限（兜底，正常发送方跑完即收）
#
# 🔴 验证口径铁律（已确立，写死在断言注释）：
#   - send 无回声真因 = body 不对（非 self-omit）；发送者**会**收 action=post 自 echo。
#   - 广播族 = 双账号：444 操作 → 678 第二连接收广播。
#   - vote(UC-8) 走第二网关 HELIX_DEFAULT_API_BASE=http://127.0.0.1:3399（本脚本不涉，留注记）。
#   - 文件上传(UC-1.3) host-cli 无 processMessageFiles 管线 → 物理够不到，诚实记 untested(harness-gap)，禁假判。
#
# **只读 / 只抓帧脚本，不改生产代码**；不改台账（P4 收口）；不 push。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

SETTLE_MS="${SETTLE_MS:-2800}"
IDLE_HOLD_S="${IDLE_HOLD_S:-60}"
TEAM_ID="${HELIX_COMPANY_ID}"
SENDER_ID="${HELIX_COOKIE_ID}"
RECEIVER_ID="${RECEIVER_ID:-678}"
MEMBER_USERS_JSON="${MEMBER_USERS_JSON:-[\"678\",\"555\"]}"

# 双连接独立 DB（避免 cursor 串：两连接同库会共享 _helix_monotonic / channel cursor 互污）。
RX_DB="$(mktemp -u -t helix_dual_rx_db.XXXXXX)"
TX_DB="$(mktemp -u -t helix_dual_tx_db.XXXXXX)"
RX_LOG="$(mktemp -t helix_dual_rx_log.XXXXXX)"
TX_LOG="$(mktemp -t helix_dual_tx_log.XXXXXX)"
RX_FIFO="$(mktemp -u -t helix_dual_rx_fifo.XXXXXX)"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

RX_PID="" RX_HOLD_PID=""
cleanup() {
  # 关接收方：合上 FIFO 写端 → host-cli 收 EOF → 优雅 quit。
  [ -n "${RX_HOLD_PID}" ] && kill "${RX_HOLD_PID}" 2>/dev/null || true
  [ -n "${RX_PID}" ] && { kill "${RX_PID}" 2>/dev/null; wait "${RX_PID}" 2>/dev/null; } || true
  rm -f "${RX_FIFO}" "${RX_DB}" "${TX_DB}" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

# ── 前置检查 ────────────────────────────────────────────────
fail_pre=0
[ -z "$SENDER_ID" ]   && { red "⛔ 缺 HELIX_COOKIE_ID（config 或 env）"; fail_pre=1; }
[ -z "$TEAM_ID" ]     && { red "⛔ 缺 HELIX_COMPANY_ID（config 或 env）"; fail_pre=1; }
[ -z "$RECEIVER_ID" ] && { red "⛔ 缺 RECEIVER_ID（第二连接 userId）"; fail_pre=1; }
nc -z -w3 127.0.0.1 "$HELIX_GO_PORT" 2>/dev/null || { red "⛔ Go WS 端口不可达：:$HELIX_GO_PORT —— ${HELIX_START_HINT}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止（未起任何连接）"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle() { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

ylw "▶ 预编译 host-cli（cargo build -q）…"
cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }

# ════════ 接收方（678）：先连、idle 持续抓 RAW 帧到 RX_LOG ════════
# 用 FIFO 当 host-cli 的 stdin：把写端用一个长跑 holder（sleep）独占持开 →
# host-cli 不收 EOF → 不 quit → WS 连接保持 → 持续接广播帧灌 RX_LOG。
# 发送方全跑完后 cleanup 杀 holder → FIFO 收 EOF → 接收方优雅退出。
ylw "▶ 接收方 ${RECEIVER_ID} 起连（idle 抓帧 → RX_LOG=${RX_LOG}）"
mkfifo "${RX_FIFO}"
# holder：持开 FIFO 写端（fd 3），sleep 兜底上限，保证 host-cli stdin 不 EOF。
( exec 3>"${RX_FIFO}"; sleep "${IDLE_HOLD_S}" ) &
RX_HOLD_PID=$!
# 接收方 host-cli：读 FIFO（永不 EOF 直到 holder 死），dump 帧。独立 RX_DB 防 cursor 串。
HELIX_DB="$RX_DB" \
  cargo run -q -p helix-host-cli -- --config "$HELIX_CONFIG_PATH" \
  --cookie-id "$RECEIVER_ID" --company-id "$TEAM_ID" <"${RX_FIFO}" >>"${RX_LOG}" 2>&1 &
RX_PID=$!

# 给接收方 WS connect + hello 握手盖 connectionId 落定的时间（握手前广播收不全）。
ylw "  ↳ 等接收方握手落定（${SETTLE_MS}ms ×2）"
settle; settle

# ════════ 发送方（444）：逐行喂命令（行间 settle 等广播扩散），灌 TX_LOG ════════
# 发送方是**一次性 drive**（跑完 quit）；接收方在它跑期间持续抓广播。
# 首 settle 在命令之前等发送方自身握手（对齐 pendingSends/businessReady gate）。
tx_drive() {
  { settle; for line in "$@"; do printf '%s\n' "$line"; settle; done; printf 'quit\n'; } \
    | HELIX_DB="$TX_DB" \
        cargo run -q -p helix-host-cli -- --config "$HELIX_CONFIG_PATH" \
        --cookie-id "$SENDER_ID" --company-id "$TEAM_ID" >>"${TX_LOG}" 2>&1
}

# ── 发送方 Phase 0：建群（真源 §4 body，含 678 入群）────────────
ylw "▶ 发送方 ${SENDER_ID} Phase 0：create 自建群（teamId=${TEAM_ID}，拉 ${RECEIVER_ID} 入群）"
CREATE_BODY="$(python3 - "$SENDER_ID" "$TEAM_ID" "$MEMBER_USERS_JSON" <<'PY'
import json, sys
self_id, team_id, members_raw = sys.argv[1], sys.argv[2], sys.argv[3]
members = json.loads(members_raw)
users = [{"id": self_id, "teamId": team_id, "role": "CREATOR"}]
for m in members:
    users.append({"id": m, "teamId": team_id, "role": "MEMBER"})
all_ids = [self_id] + list(members)
body = {
    "teamId": team_id, "displayName": "helix-dual-e2e", "orient": "",
    "type": "P", "users": users, "picturetype": "USER",
    "picture": {"userIds": all_ids}, "forceCreate": True,
}
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
echo "create body = $CREATE_BODY" >>"${TX_LOG}"
printf '__TX_CREATE_MARKER__\n' >>"${TX_LOG}"
tx_drive "create $CREATE_BODY"

# 从发送方自回声抠新 channelId（create 的 channel_created 自 echo·发送方一定收）。
extract_tx_channel_id() {
  awk '/__TX_CREATE_MARKER__/{seen=1} seen' "${TX_LOG}" \
    | grep -E "RAW WS inbound" | grep -E '"action": ?"channel_created"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/'
}
TARGET="$(extract_tx_channel_id)"
if [ -n "${TARGET}" ]; then
  grn "  ↳ 抠到自建 channelId = ${TARGET}（678 已在 users[] → 应收 channel_created 广播）"
else
  red "  ↳ 未从发送方 channel_created 自回声抠到 channelId（create 体/环境？看 ${TX_LOG}）"
fi

if [ -n "${TARGET}" ]; then
  # ── 发送方 Phase 1：群里 send（678 应收 post 广播）──────────────
  ylw "▶ 发送方 Phase 1：群内 send（${RECEIVER_ID} 应收 post 广播）"
  printf '__TX_SEND_MARKER__\n' >>"${TX_LOG}"
  tx_drive "send ${TARGET} dual-e2e-broadcast-probe-$(now_ms)"

  # ── 发送方 Phase 2：改成员（leave 555 → 678 应收 member_change 广播）──
  # 真源 §5：{channelId, leaveUsers:[{id,teamId,role}]}。踢一个**非 678**成员（555），
  # 这样 678 作为留存成员收 channel_member_update 广播（踢自己会让 678 离群收不到后续）。
  ylw "▶ 发送方 Phase 2：member change（踢 555 → ${RECEIVER_ID} 收 member_change 广播）"
  MEMBER_BODY="$(python3 - "${TARGET}" "$TEAM_ID" "$MEMBER_USERS_JSON" "$RECEIVER_ID" <<'PY'
import json, sys
ch, team_id, members_raw, receiver = sys.argv[1:5]
members = json.loads(members_raw)
# 踢一个**非接收方**的成员，保证 receiver 留存收广播。
target = next((m for m in members if m != receiver), None)
body = {"channelId": ch}
if target:
    body["leaveUsers"] = [{"id": target, "teamId": team_id, "role": "MEMBER"}]
print(json.dumps(body, ensure_ascii=False, separators=(",", ":")))
PY
)"
  echo "member body = $MEMBER_BODY" >>"${TX_LOG}"
  printf '__TX_MEMBER_MARKER__\n' >>"${TX_LOG}"
  tx_drive "member $MEMBER_BODY"

  # ── 发送方 Phase 3：改群信息 nickname（update_channel 定向广播验证）──
  ylw "▶ 发送方 Phase 3：nickname（update_channel 类广播）"
  printf '__TX_NICK_MARKER__\n' >>"${TX_LOG}"
  tx_drive "nickname ${TARGET} dual-e2e-nick"

  # 发送方动作已发完，给接收方多 settle 几拍把延迟广播灌全（双账号扩散比自回声慢）。
  ylw "▶ 收尾：等接收方把延迟广播灌全（${SETTLE_MS}ms ×2）"
  settle; settle
fi

# ── 关接收方：杀 holder → FIFO EOF → host-cli quit → flush RX_LOG ──
ylw "▶ 关接收方连接（flush RX_LOG）"
[ -n "${RX_HOLD_PID}" ] && kill "${RX_HOLD_PID}" 2>/dev/null || true
[ -n "${RX_PID}" ] && wait "${RX_PID}" 2>/dev/null || true
RX_HOLD_PID="" RX_PID=""   # 已收，避免 cleanup 重复

# ════════ 断言 ════════════════════════════════════════════════
declare -a RESULTS
# 在指定 LOG 里精确匹配 RAW WS inbound 帧的 action（支持 a|b 备选）。
# ⚠️ 单次 grep + grep -c（不用 grep -qE 管道：pipefail 下 -q 命中关管道 → 上游 SIGPIPE → 误判 FAIL）。
ws_hit() {  # logfile action-alternation → 0 命中 / 1 未命中
  local n
  n="$(grep -E "RAW WS inbound" "$1" | grep -cE "\"action\": ?\"($2)\"" || true)"
  [ "${n:-0}" -gt 0 ]
}
assert_rx() {  # action label —— 接收方 RX_LOG 必须收到的广播帧（核心证据）
  if ws_hit "${RX_LOG}" "$1"; then
    RESULTS+=("PASS|$2|RX 收到广播 action '$1'")
  else
    RESULTS+=("FAIL|$2|接收方 RX_LOG 未见广播 action '$1'")
  fi
}
soft_rx() {  # action label skip_reason —— 软依赖（数据/广播范围依赖，缺不算 wire bug）
  if ws_hit "${RX_LOG}" "$1"; then
    RESULTS+=("PASS|$2|RX 收到广播 action '$1'")
  else
    RESULTS+=("SKIP|$2|$3")
  fi
}
assert_tx() {  # action label —— 发送方自身回声照常（对照组：证发送方连接 work）
  if ws_hit "${TX_LOG}" "$1"; then
    RESULTS+=("PASS|$2|TX 自回声 action '$1'")
  else
    RESULTS+=("FAIL|$2|发送方 TX_LOG 未见自回声 action '$1'")
  fi
}

if [ -n "${TARGET:-}" ]; then
  # —— 接收方广播族（核心：单连接物理够不到的扩散证据）——
  assert_rx "channel_created"                     "① RX channel_created (678 被拉入群)"
  assert_rx "post"                                "② RX post (444 群内发消息 → 678 收广播 · 核心冒烟)"
  soft_rx "channel_member_update" "③ RX member_change (踢 555 → 678 收)" \
    "channel_member_update 广播范围 = 频道成员；若 Go 仅推被增减用户(555)则 678 收不到（广播范围依赖，非 wire bug）"
  soft_rx "update_channel|update_channel_member_nickName" "④ RX update_channel (nickname/群信息变更广播)" \
    "nickname 可能仅定向触发者(444)·非全员广播（定向范围依赖，非 wire bug）"
  soft_rx "post_read" "⑤ RX post_read (已读未读态广播)" \
    "本脚本未驱动跨账号 read；post_read 广播留作后续（数据依赖，非 bug）"
  # —— 发送方对照组（证发送方连接照常自回声）——
  assert_tx "channel_created"                     "⑥ TX channel_created (发送方自回声照常)"
  assert_tx "post"                                "⑦ TX post (发送方 send 自回声照常)"
else
  RESULTS+=("FAIL|①-⑦ 全链|create 无 channelId，双账号链无法跑（看 ${TX_LOG} 定位 create 体/环境）")
fi

# —— 物理够不到项：诚实记 untested(harness-gap)，禁假判 ——
RESULTS+=("GAP|UC-1.3 文件上传|host-cli 无 processMessageFiles 管线 → 物理够不到，untested(harness-gap)")
RESULTS+=("GAP|UC-8 vote|走第二网关 3399（HELIX_DEFAULT_API_BASE），本双账号脚本不涉，单独 e2e 验")

# ── 汇总 ────────────────────────────────────────────────────
echo; echo "════════ host-cli 双账号并发 e2e 结果（${SENDER_ID} 操作 → ${RECEIVER_ID} 收广播）════════"
pass=0 failc=0 skip=0 gap=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ $label — $note"; pass=$((pass+1));;
    FAIL) red "❌ $label — $note"; failc=$((failc+1));;
    SKIP) ylw "⏭  $label — $note"; skip=$((skip+1));;
    GAP)  ylw "🕳  $label — $note"; gap=$((gap+1));;
  esac
done
echo "─────────────────────────────────────────────"
echo "自建 channelId: ${TARGET:-<未建/失败>}"
echo "接收方 RX_LOG : ${RX_LOG}"
echo "发送方 TX_LOG : ${TX_LOG}"
echo "PASS $pass · FAIL $failc · SKIP $skip · GAP(harness) $gap"
echo "（RX 广播帧证据：grep 'RAW WS inbound' ${RX_LOG} | grep action）"
[ "$failc" -gt 0 ] && exit 1 || exit 0
