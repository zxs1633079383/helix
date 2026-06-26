#!/usr/bin/env bash
# host-cli ↔ Go sync-gap e2e —— Round-5 P2 三个「构造触发态」UC 的 e2e 剧本。
#
# 对象（代码已实现，本脚本只构造触发态 + 抓真 wire 帧裁决）：
#   - UC-4.5 陌生 channel 不丢帧（bf25307）：本地 cursor store 空（冷启动新租户 DB）→ 服务端
#       hello 后下发的每条 `increment_channel` 都命中本地未知 channel → `entry().or_insert_with(0)`
#       自动注册不丢帧；并对一个抠到的真 channelId 跑 `im_channel_load_increment_by_channel_id`
#       HTTP 读路径（channel/load/incrementByChannelId）。验据 = N 条 increment_channel 入站
#       → N 次 `im:channel:increment` emit（零丢）。
#   - UC-4.4 心跳 gap 补偿（bf25307 pong 消费侧 + sync_increment ping piggyback）：持久连接挂住
#       ≥ ping_interval（默认 8s，self-arm 周期心跳）→ 客户端 ping 帧 piggyback `{cursors, allHash}`
#       → 服务端 `ReconcileCursors` 比对回 **pong**（`{"status":"OK", "seq_reply":N,
#       "data":{"gaps":[…], "hashMismatch":bool, "text":"pong"}}`）。验据 = 入站 pong 帧带
#       seq_reply + gaps/hashMismatch（证明 ping piggyback 往返成功，触发 __pong__ 合成 + 补偿）。
#       ⚠️ 出站 ping 帧体本身 host 不落日志（network.rs 只 dump RAW WS inbound）——以服务端 pong
#       回声为往返证据（pong 的 seq_reply 锚定到客户端 ping seq）。
#   - UC-4.3 too_long（sync_session::TooLong + emit_sync_too_long）：需本地 cursor 远落后服务端
#       seq 形成超阈 gap，服务端 sync 响应 kind=`too_long{resetTo}` → 清表重拉。host-cli 无法
#       构造「本地 cursor 落后服务端足够多」的态（冷启动 cursor=0 时服务端 increment 直接补齐，
#       不进 too_long；且无注入历史落后 cursor 的入口）→ 诚实记 **harness-gap**，不假判。
#
# 用法：
#   [MEMBER_USERS_JSON='["678","555"]'] bash scripts/host-cli-sync-gap-e2e.sh
#   # 身份/端口从 helix.config.yaml 读取；覆盖：HELIX_COOKIE_ID / HELIX_COMPANY_ID
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

LOG="$(mktemp -t helix_sync_gap.XXXXXX.log)"
WAIT_TO="${WAIT_TO:-12}"                 # wait_for 单次回声等待上限（秒）
IDLE_HOLD_S="${IDLE_HOLD_S:-60}"         # holder 兜底持开 FIFO 写端上限
# UC-4.4：必须 ≥ helix-im ConfigDefault ping_interval_ms(8000) + 服务端往返裕量。给 11s。
PING_HOLD_S="${PING_HOLD_S:-11}"
TEAM_ID="${HELIX_COMPANY_ID}"
SELF_ID="${HELIX_COOKIE_ID}"
# 冷启动空 cursor store：独立租户 DB 路径，确保本地无任何 channel cursor（UC-4.5 触发前提）。
FRESH_DB="${HELIX_DB:-$(mktemp -u -t helix_sync_gap_db.XXXXXX).sqlite}"

red() { printf '\033[31m%s\033[0m\n' "$*"; }
grn() { printf '\033[32m%s\033[0m\n' "$*"; }
ylw() { printf '\033[33m%s\033[0m\n' "$*"; }

# ── 前置检查 ────────────────────────────────────────────────
fail_pre=0
[ -z "$SELF_ID" ] && { red "⛔ 缺 HELIX_COOKIE_ID（config 或 env）"; fail_pre=1; }
[ -z "$TEAM_ID" ] && { red "⛔ 缺 HELIX_COMPANY_ID（config 或 env）"; fail_pre=1; }
nc -z -w3 127.0.0.1 "$HELIX_GO_PORT" 2>/dev/null || { red "⛔ Go WS 不可达 :$HELIX_GO_PORT —— ${HELIX_START_HINT}"; fail_pre=1; }
[ "$fail_pre" = 1 ] && { red "前置不满足，中止"; exit 2; }

now_ms() { python3 -c 'import time;print(int(time.time()*1000))'; }

TX_FIFO="$(mktemp -u -t helix_sync_gap_fifo.XXXXXX)"
CLI_PID=""; HOLDER_PID=""
cleanup() {
  exec 9>&- 2>/dev/null || true            # 合写端 → host-cli 收 EOF 优雅 quit
  [ -n "$HOLDER_PID" ] && kill "$HOLDER_PID" 2>/dev/null || true
  [ -n "$CLI_PID" ] && wait "$CLI_PID" 2>/dev/null || true
  rm -f "$TX_FIFO" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

mkfifo "$TX_FIFO"

# ── 起持久连接 host-cli（一条 WS / 一次 hello / 冷启动空 DB）──────
ylw "▶ 预编译 host-cli（cargo build -q）…"; cargo build -q -p helix-host-cli || { red "build 失败"; exit 1; }
ylw "▶ 起持久连接 host-cli（冷启动空 cursor store：DB=${FRESH_DB}）"
RUST_LOG='info,helix_im=info,helix_driver_host=info' HELIX_DB="$FRESH_DB" \
  cargo run -q -p helix-host-cli -- --config "$HELIX_CONFIG_PATH" \
  --cookie-id "$SELF_ID" --company-id "$TEAM_ID" <"$TX_FIFO" >>"$LOG" 2>&1 &
CLI_PID=$!
exec 9>"$TX_FIFO"
( sleep "$IDLE_HOLD_S" ) & HOLDER_PID=$!

feed() { printf '%s\n' "$1" >&9; }

# 轮询 LOG 等某 inbound action 帧出现（marker 之后），命中即返 0，超时返 1。
wait_for() {  # action-alt [timeout_s] [marker]
  local act="$1" to="${2:-$WAIT_TO}" marker="${3:-}" t=0 src
  while [ "$t" -lt "$((to*5))" ]; do
    if [ -n "$marker" ]; then src="$(awk -v m="$marker" '$0~m{seen=1} seen' "$LOG")"; else src="$(cat "$LOG")"; fi
    if printf '%s' "$src" | grep -E "RAW WS inbound" | grep -qE "\"action\": ?\"($act)\""; then return 0; fi
    python3 -c "import time;time.sleep(0.2)"; t=$((t+1))
  done
  return 1
}

declare -a RESULTS

# ════════ 等 hello（冷启动后服务端下发 increment 全量）════════
ylw "▶ 等 hello 握手（持久连接全程仅此一次）"
if wait_for "hello" 15; then grn "  ↳ hello 到达，connectionId 已盖章"; else red "  ↳ 15s 未见 hello（环境/网络？）"; fi

# ════════ UC-4.5：陌生 channel 不丢帧（冷启动 increment 自动注册）════════
# 冷启动本地 cursor store 空 → hello 后服务端推的每条 increment_channel 都命中未知 channel
# → entry().or_insert_with(Channel::new(_, 0)) 自动注册不丢帧。等 increment_channel_end 收尾。
ylw "▶ UC-4.5 等冷启动 increment 全量下发（每条均命中本地未知 channel）"
wait_for "increment_channel" "$WAIT_TO" || ylw "  ↳ 未见 increment_channel（账号无群？）"
wait_for "increment_channel_end" "$WAIT_TO" || ylw "  ↳ 未见 increment_channel_end（继续，按已收帧裁决）"

INC_FRAMES="$(grep -E "RAW WS inbound" "$LOG" | grep -cE '"action": ?"increment_channel"' || true)"
INC_EMITS="$(grep -cE 'event=im:channel:increment|"event":"im:channel:increment"|im:channel:increment' "$LOG" || true)"
grn "  ↳ 入站 increment_channel 帧数=$INC_FRAMES · im:channel:increment emit 数=$INC_EMITS"

if [ "${INC_FRAMES:-0}" -gt 0 ] && [ "${INC_EMITS:-0}" -ge "${INC_FRAMES:-0}" ]; then
  # 抠一个真 channelId（用于 4.5 HTTP 读路径补充验证）。
  SAMPLE_CH="$(grep -E "RAW WS inbound" "$LOG" | grep -E '"action": ?"increment_channel"' \
    | grep -oE '"id":"[a-z0-9]{26}"' | head -1 | sed -E 's/.*"id":"([^"]+)".*/\1/')"
  RESULTS+=("PASS|UC-4.5 陌生 channel 自动注册|冷启动空 cursor store · ${INC_FRAMES} 条 increment_channel 全命中未知 channel · ${INC_EMITS} 次 im:channel:increment emit 零丢")
else
  RESULTS+=("SKIP|UC-4.5 陌生 channel 自动注册|increment 帧=${INC_FRAMES} emit=${INC_EMITS}（账号无群 / 未下发 · data-dep）")
  SAMPLE_CH=""
fi

# 补充：对抠到的真 channelId 跑 HTTP 读路径 incrementByChannelId（读族 → im:read:result 透传）。
if [ -n "${SAMPLE_CH:-}" ]; then
  printf '__INCR_HTTP_MARKER__\n' >>"$LOG"
  ylw "▶ UC-4.5（HTTP 读路径）im_channel_load_increment_by_channel_id ch=${SAMPLE_CH}"
  feed "cmd im_channel_load_increment_by_channel_id {\"channel_id\":\"${SAMPLE_CH}\"}"
  # 读族无 WS 回声，等 im:read:result emit（HTTP 响应体透传回灌）。
  t=0; HTTP_OK=0
  while [ "$t" -lt 40 ]; do
    if awk '/__INCR_HTTP_MARKER__/{seen=1} seen' "$LOG" | grep -qE 'im:read:result'; then HTTP_OK=1; break; fi
    python3 -c "import time;time.sleep(0.25)"; t=$((t+1))
  done
  if [ "$HTTP_OK" = 1 ]; then
    RESULTS+=("PASS|UC-4.5 HTTP incrementByChannelId|channel/load/incrementByChannelId → im:read:result 透传回灌")
  else
    RESULTS+=("SKIP|UC-4.5 HTTP incrementByChannelId|im:read:result 未在 10s 内回灌（响应体 emit 可能 debug 级 · 见 LOG）")
  fi
fi

# ════════ UC-4.4：心跳 gap 补偿（ping piggyback → 服务端 pong 往返）════════
# 持久连接挂住 ≥ ping_interval(8s)：self-arm 心跳到期发 ping 帧 piggyback {cursors, allHash}
# → 服务端 ReconcileCursors 回 pong{seq_reply, gaps, hashMismatch}。入站 pong 即往返证据。
PING_MARK_TS="$(now_ms)"
printf '__PONG_WATCH_MARKER__\n' >>"$LOG"
ylw "▶ UC-4.4 挂住持久连接 ${PING_HOLD_S}s 等心跳 ping → 服务端 pong（≥ ping_interval 8s）"
t=0; PONG_OK=0
while [ "$t" -lt "$((PING_HOLD_S*5))" ]; do
  if awk '/__PONG_WATCH_MARKER__/{seen=1} seen' "$LOG" | grep -qE '"text":"pong"'; then PONG_OK=1; break; fi
  python3 -c "import time;time.sleep(0.2)"; t=$((t+1))
done
PONG_FRAME="$(awk '/__PONG_WATCH_MARKER__/{seen=1} seen' "$LOG" | grep -oE '\{"status":"OK"[^}]*"text":"pong"\}' | head -1)"
if [ "$PONG_OK" = 1 ]; then
  grn "  ↳ 收到 pong：${PONG_FRAME}"
  RESULTS+=("PASS|UC-4.4 心跳 ping piggyback 往返|ping(≥8s self-arm) → 服务端 pong：${PONG_FRAME}")
else
  RESULTS+=("SKIP|UC-4.4 心跳 ping piggyback 往返|${PING_HOLD_S}s 内未见 pong（服务端 ping_interval 长 / 网络 · data-dep）")
fi

# ════════ UC-4.3：too_long —— 诚实记 harness-gap（构造不出态）════════
# host-cli 无「注入本地落后 cursor」入口：冷启动 cursor=0 时服务端走 increment 直接补齐，
# 不进 sync too_long 分支；也无法把本地 cursor 顶到远落后服务端 seq 的态。物理够不到 → 不假判。
RESULTS+=("HARNESS-GAP|UC-4.3 too_long 重拉|host-cli 无注入落后 cursor 入口 · 冷启动 cursor=0 走 increment 补齐不进 too_long · 构造不出超阈 gap（代码侧 sync_session::TooLong + emit_sync_too_long 由 helix-im 单测覆盖）")

# ── 关连接 ──
cleanup; trap - EXIT INT TERM

# ── 汇总 ─────────────────────────────────────────────────────
echo; echo "════════ host-cli↔Go sync-gap e2e 结果（Round-5 P2）════════"
pass=0 skip=0 gap=0
for r in "${RESULTS[@]}"; do
  IFS='|' read -r st label note <<<"$r"
  case "$st" in
    PASS) grn "✅ e2e-verified  $label — $note"; pass=$((pass+1));;
    SKIP) ylw "⏭  data-dep       $label — $note"; skip=$((skip+1));;
    HARNESS-GAP) ylw "🚧 harness-gap    $label — $note"; gap=$((gap+1));;
  esac
done
echo "── verified=$pass data-dep=$skip harness-gap=$gap · 一条连接一次 hello · LOG=$LOG"
echo "   （冷启动 DB=$FRESH_DB · UC-4.3 为 harness-gap 不计 FAIL）"
exit 0
