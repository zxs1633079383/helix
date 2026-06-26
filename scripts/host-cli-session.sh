#!/usr/bin/env bash
# host-cli 单连接 ping-pong 保活驱动（#18 重构，替代「每命令起一次 host-cli」）。
#
# 原浪费：`{ sleep;echo cmd; } | host-cli` 每条命令一次 host-cli → 每次重连 + 重拉 increment(45ch)+sync。
# 本驱动：mkfifo 占住 stdin 写端 → host-cli **一次** connect+increment → 命令逐条喂同一连接 →
#         host-cli 自带心跳 ping(8s) ping-pong 保活 → 全程零重连。更快、更贴近真实长连客户端。
#
# 用法：命令列表经 stdin（每行一条），或管道：
#   printf 'read CH\nnickname CH 新名\nsend CH hi\n' | scripts/host-cli-session.sh
# env：SETTLE_SEC(默认5) / WARMUP_SEC(默认7) / HLOG / HELIX_DB
#      身份/端口从 helix.config.yaml 读取（覆盖：HELIX_COOKIE_ID / HELIX_COMPANY_ID）
set -uo pipefail
cd "$(dirname "$0")/.."

# ── 读配置（helix.config.yaml）───────────────────────────────────────────
# shellcheck source=lib-config.sh
source "$(dirname "$0")/lib-config.sh"

GOLOG="${HELIX_GO_LOG}"
SETTLE="${SETTLE_SEC:-5}"; WARMUP="${WARMUP_SEC:-7}"
export RUST_LOG="${RUST_LOG:-info,helix_im=debug,helix_driver_host=debug}"
TS=$(date +%s)
export HELIX_DB="${HELIX_DB:-/tmp/helix_session_${TS}.db}"
HLOG="${HLOG:-/tmp/helix_session_${TS}.log}"

lsof -nP -iTCP:"$HELIX_GO_PORT" -sTCP:LISTEN -t >/dev/null 2>&1 || { echo "⛔ Go :$HELIX_GO_PORT 未监听 — ${HELIX_START_HINT}"; exit 2; }
echo "▶ 构建 host-cli"; cargo build -p helix-host-cli --quiet || { echo "⛔ build 失败"; exit 1; }
GOBASE=$(wc -l < "$GOLOG" 2>/dev/null || echo 0)

FIFO=$(mktemp -u /tmp/helix_fifo.XXXXXX); mkfifo "$FIFO"
# 单连接铁律：先起 reader（host-cli < fifo，open 阻塞等 writer），再 exec 3>fifo 占住写端解锁。
# 占住写端期间 host-cli stdin 永不 EOF → 连接靠心跳 ping-pong 保活，命令逐条喂。
timeout 180 ./target/debug/helix-host-cli --config "$HELIX_CONFIG_PATH" < "$FIFO" > "$HLOG" 2>&1 &
HPID=$!
exec 3>"$FIFO"
echo "▶ 等 ${WARMUP}s：一次 connect+hello+increment(45ch)+sync 落定（全程仅此一次）"
sleep "$WARMUP"

n=0
while IFS= read -r cmd; do
  [ -z "$cmd" ] && continue
  n=$((n+1)); echo "  >> [$n] $cmd"
  printf '%s\n' "$cmd" >&3
  sleep "$SETTLE"
done
echo "  >> quit（关写端 → host-cli EOF 优雅退出）"
printf 'quit\n' >&3
exec 3>&-
wait "$HPID" 2>/dev/null
rm -f "$FIFO"

echo ""
echo "════ connect 次数（单连接铁律：必须 == 1）════"
grep -c '主 WS 已连接' "$HLOG"
echo "════ ③ 收帧统计 ════"; grep -oE '"action": ?"[a-z_]+"' "$HLOG" | sort | uniq -c
echo "════ ① emit 统计 ════"; grep -oE 'DomainEvent emit event=im:[a-z:_]+' "$HLOG" | sort | uniq -c
echo "════ 心跳 ping（保活证据）════"; grep -c 'ping_frame\|ping.*seq\|heartbeat' "$HLOG"
echo "════ ② Go delta 驱动的命令 ════"
tail -n +$((GOBASE+1)) "$GOLOG" 2>/dev/null | grep -oE '"方法":"[a-zA-Z]+"' | sort | uniq -c
echo ""
echo "产物：helix log=$HLOG  DB=\"${HELIX_DB}?mode=rwc\""
