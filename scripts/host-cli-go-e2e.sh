#!/usr/bin/env bash
# host-cli ↔ Go 13-action e2e 驱动 —— 真连 Go 验证「共享 driver-host I/O 栈 + 全 WS action」。
#
# 定位（见根 CLAUDE.md §「host-cli 是什么」）：验的是**共享 helix-driver-host 的 WS/HTTP 栈 + wire 契约**，
# 跑一次即可——native/ffi 复用同一栈，端的增量靠 smoke.cpp(边界) + F5(parity) 覆盖，不重跑本套。
#
# 前置（外部状态，脚本起不了，须你先起）：
#   - Go mattermost :8065 + Java paas/x9 grpc(xpa-2-12-grpc) UP（见 SESSION §0 grpc 根因文档）
#   - 一个真 channelId（冷启动落库的 257 之一）作非破坏操作靶
# 用法：
#   CHANNEL_ID=<真channelId> HELIX_COOKIE_ID=<userId> HELIX_COMPANY_ID=<companyId> \
#     bash scripts/host-cli-go-e2e.sh
# 可选 env：
#   CLOSE_CHANNEL_ID=<另一个可销毁channelId>   # 设了才跑 leave/close（破坏性，默认跳过防误删主靶）
#   CREATE_BODY='{"displayName":"e2e","type":"P","users":[{"id":"<uid>","name":"x","role":"MEMBER","teamId":""}]}'
#                                              # 设了才跑 create（需现网成员对象，脚本不臆造）
#   HELIX_WS_URL / HELIX_API_BASE              # 默认 ws://127.0.0.1:8065/api/v4/websocket·http://127.0.0.1:8065/api/cses
#   SETTLE_MS=2500                             # 每命令后等 WS 回推的毫秒（慢网调大）
#
# 断言：grep host-cli 日志里 `HELIX_DUMP_FRAMES` 的 `RAW WS inbound … frame=<Go JSON>` 行的 WS action 字符串
#       （Go 真推 = ground truth）+ 旁证 `DomainEvent emit event=im:*`（frame→handler→emit 全管道）。
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
LOG="$(mktemp -t helix_e2e.XXXXXX.log)"
SETTLE_MS="${SETTLE_MS:-2500}"
WS_URL="${HELIX_WS_URL:-ws://127.0.0.1:8065/api/v4/websocket}"
API_BASE="${HELIX_API_BASE:-http://127.0.0.1:8065/api/cses}"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

# ── 前置检查 ────────────────────────────────────────────────
fail_pre=0
if [ -z "${CHANNEL_ID:-}" ]; then red "⛔ 缺 CHANNEL_ID（真 channelId 作非破坏操作靶）"; fail_pre=1; fi
if [ -z "${HELIX_COOKIE_ID:-}" ]; then red "⛔ 缺 HELIX_COOKIE_ID（=userId 身份头，缺则 Go 判 session 过期）"; fail_pre=1; fi
# Go 可达性：用 mattermost 健康端点 /api/v4/system/ping（200=活）。
# ⚠️ 不用 `curl -f` 测根路径——根返 404（正常），-f 会把 404 误判为 down。
http_root="${API_BASE%/api/cses}"
ping_code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 "$http_root/api/v4/system/ping" 2>/dev/null)"
if [ "$ping_code" != "200" ]; then
  # 兜底：根路径有任何 HTTP 响应（含 404）即算服务在监听（仅 000/空 = 真连不上）。
  root_code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 "$http_root/" 2>/dev/null)"
  if [ -z "$root_code" ] || [ "$root_code" = "000" ]; then
    red "⛔ Go 不可达：${http_root}（ping=$ping_code root=${root_code}）—— 先起 mattermost :8065 + grpc(xpa-2-12-grpc)"; fail_pre=1
  else
    ylw "⚠️ ping 非 200（=${ping_code}）但根有响应（${root_code}）→ 服务在监听，继续"
  fi
fi
[ "$fail_pre" = 1 ] && { red "前置不满足，中止（未跑任何命令）"; exit 2; }

# Pulsar 可达性（软警告，不中止）：posts/create 是 **Pulsar 异步落库** 路径（full-map #1）；
# Pulsar down → send/posts_update/revoke/makeTopic 无回声（非代码 bug）。
# 同步类（nickname/schedule/cancel/close/leave）不经 Pulsar，不受影响。
if ! nc -z -w2 127.0.0.1 6650 2>/dev/null; then
  ylw "⚠️ Pulsar :6650 不通 —— send/posts_update/revoke/makeTopic（消息创建族，走 Pulsar 异步）将无回声。"
  ylw "   先起 Pulsar 再验消息族；本次仍跑同步类（nickname/schedule/cancel）做连通性验证。"
fi

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }
settle() { python3 -c "import time;time.sleep(${SETTLE_MS}/1000)"; }

# 跑一组命令（逐行喂 host-cli，行间 settle 等 WS 回推），append 到 $LOG。
# host-cli 读 stdin 到 quit/EOF；HELIX_DUMP_FRAMES=1 打印入站帧。
drive() {
  # ⚠️ 首个 settle 在命令**之前**：等 WS connect + hello 握手盖 connectionId 落定，
  # 否则第一条命令（如 send）会在 connectionId=None 时发 HTTP → Go 401/403 无回推
  #（对齐现网前端 pendingSends/businessReady gate：握手前不发业务请求）。
  { settle; for line in "$@"; do printf '%s\n' "$line"; settle; done; printf 'quit\n'; } \
    | HELIX_DUMP_FRAMES=1 HELIX_WS_URL="$WS_URL" HELIX_API_BASE="$API_BASE" \
      cargo run -q -p helix-host-cli >>"$LOG" 2>&1
}

# 断言：日志里 RAW WS inbound 帧的 `"action": "<exact>"` = Go 真推了该 action。
# ⚠️ 精确匹配引号内整值（非裸子串）——否则 "update_channel" 会被 "update_channel_member_nickName" 误命中。
# action 支持 `a|b` 备选（如 posted|post）。
declare -a RESULTS
assert_ws() {
  local action="$1" label="$2"
  if grep -E "RAW WS inbound" "$LOG" | grep -qE "\"action\": ?\"($action)\""; then
    RESULTS+=("PASS|$label|WS action '$action'")
  else
    RESULTS+=("FAIL|$label|未见 WS action '$action'（Go 未推/未连/命令未达）")
  fi
}

ylw "▶ 预编译 host-cli（cargo build -q）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ Phase 1：非破坏 channel-scoped 命令（靶 channel=${CHANNEL_ID}）"
TS_FUTURE="$(( $(now_ms) + 600000 ))"   # 定时消息：10 分钟后
drive \
  "send $CHANNEL_ID e2e-probe-$(now_ms)" \
  "read $CHANNEL_ID $(now_ms)" \
  "nickname $CHANNEL_ID e2e-nick" \
  "schedule $CHANNEL_ID $TS_FUTURE e2e-scheduled" \
  "cancel $CHANNEL_ID"

assert_ws "posted|post"                    "① post/posted-echo (send)"
assert_ws "posts_update"                   "② posts_update (send 旁推)"
assert_ws "post_read"                      "③ post_read (read)"
assert_ws "update_channel"                 "④ update_channel (read 旁推)"
assert_ws "update_channel_member_nickName" "⑤ nickname（注意 camelCase N）"
assert_ws "post_schedule_created"          "⑥ schedule"
assert_ws "post_schedule_canceled"         "⑦ cancel"

# ── Phase 2：postId 依赖（revoke / makeTopic）—— 从 Phase 1 posted 帧抠 server postId ──
ylw "▶ Phase 2：抠 postId 链式（revoke / makeTopic）"
# 只从 send 的 posted 回声帧抠 server postId（非任意 increment 帧的 lastPost.id）。
POST_ID="$(grep -E "RAW WS inbound" "$LOG" | grep -E '"action": ?"posted"' \
  | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
if [ -n "$POST_ID" ]; then
  drive "revoke $POST_ID" "makeTopic {\"postId\":\"$POST_ID\",\"displayName\":\"e2e-topic\",\"type\":\"P\"}"
  assert_ws "post_update"     "⑧ post_update (revoke)"
  assert_ws "channel_created" "⑨ channel_created (makeTopic)"
else
  RESULTS+=("SKIP|⑧⑨ revoke/makeTopic|未从 posted 帧抠到 postId（send 是否成功？看 ${LOG}）")
fi

# ── 可选：create（需现网成员对象 JSON）──
if [ -n "${CREATE_BODY:-}" ]; then
  ylw "▶ create（CREATE_BODY 已给）"
  drive "create $CREATE_BODY"
  assert_ws "channel_created" "⑩ channel_created (create)"
else
  RESULTS+=("SKIP|⑩ create|未给 CREATE_BODY（需现网成员对象，脚本不臆造）")
fi

# ── 可选：破坏性 leave/close（仅当给了可销毁 channel）──
if [ -n "${CLOSE_CHANNEL_ID:-}" ]; then
  ylw "▶ 破坏性 leave/close（靶 channel=${CLOSE_CHANNEL_ID}）"
  drive "leave $CLOSE_CHANNEL_ID" "close $CLOSE_CHANNEL_ID"
  assert_ws "channel_member_update" "⑪ channel_member_update + leave 删行 (leave)"
  assert_ws "channel_close"         "⑫ channel_close (close)"
else
  RESULTS+=("SKIP|⑪⑫ leave/close|未给 CLOSE_CHANNEL_ID（破坏性，防误删主靶）")
fi

# ── 汇总 ────────────────────────────────────────────────────
echo; echo "════════ host-cli↔Go 13-action e2e 结果 ════════"
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
echo "PASS $pass · FAIL $failc · SKIP $skip · 日志 $LOG"
echo "（旁证可看 emit：grep 'DomainEvent emit' ${LOG}）"
[ "$failc" -gt 0 ] && exit 1 || exit 0
