#!/usr/bin/env bash
# full-map 完整性闸 —— cses 全量纯 helix 化的「100% 覆盖」对账。
# 三表:① 84 HTTP(排除 search + createUser/status goBlocked) ② 19 WS ③ ~37 UC(排除 search 集合七)。
# 退出码:0 = 三表全 100% 覆盖(可判 DONE);1 = 未 100%(未完成,workflow 接力续);2 = 环境异常。
#
# 覆盖判定(静态,粗粒度信号——精确生产验证由 UC 真 Go e2e 给):
#   HTTP 覆盖 = helix outbound `distributed_slice(OUTBOUND_COMMANDS)` 注册的命令端点数 vs full-map HTTP 全集
#   WS 覆盖   = helix `ws/handlers/` 处理的 action vs full-map 19 事件
#   UC 覆盖   = UC 台账 docs/review/uc-coverage-ledger.md 里标 ✅e2e-verified 的 UC 数 vs ~37
#
# 用法:bash scripts/fullmap-coverage.sh   (workflow Phase 3 每轮跑;CI/pre-cutover 可挂)

set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PARTIALS="crates/helix-im/docs/csesapi-migration/full-map/partials"
LEDGER="docs/review/uc-coverage-ledger.md"

red()  { printf '\033[31m%s\033[0m\n' "$*"; }
grn()  { printf '\033[32m%s\033[0m\n' "$*"; }
ylw()  { printf '\033[33m%s\033[0m\n' "$*"; }

[ -d "$PARTIALS" ] || { red "⛔ 缺 full-map partials 目录"; exit 2; }

# ── ① HTTP:helix 注册的 outbound 命令端点数（distributed_slice 注册的 build 返回 endpoint）──
http_registered=$(grep -rhoE 'Ok\(\("[a-z][a-z/_-]+"' crates/helix-im/src/outbound/ 2>/dev/null \
  | grep -oE '"[a-z][a-z/_-]+"' | sort -u | grep -vE 'search|createUser|/status' | wc -l | tr -d ' ')
# full-map HTTP 全集（posts+channel+user-misc 的真实端点行,排除 bot/search/goBlocked）
http_total=$(grep -rhoE '`/(posts?|channel|channels|user|team)[a-z/A-Z]+`' \
  "$PARTIALS"/1--backend-http-posts.md "$PARTIALS"/2--backend-http-channel-sync.md "$PARTIALS"/3--backend-http-user-misc.md 2>/dev/null \
  | sort -u | grep -viE 'search|createUser|/status' | wc -l | tr -d ' ')

# ── ② WS:helix 处理的 action vs 19 ──
ws_handled=$(grep -rhoE '"(post|posted|posts_update|post_read|post_update|channel_created|channel_member_update|channel_member_role_updated|update_channel|update_channel_notice|change_channel_approval|increment_channel|increment_channel_end|post_schedule_created|post_schedule_canceled|add_followers|add_mentions|channels)"' \
  crates/helix-im/src/ 2>/dev/null | sort -u | wc -l | tr -d ' ')
ws_total=19

# ── ③ UC:台账 ✅e2e-verified vs ~37 ──
uc_total=37
uc_verified=0
[ -f "$LEDGER" ] && uc_verified=$(grep -cE '✅ *e2e-verified|UC-.* ✅' "$LEDGER" 2>/dev/null || echo 0)

echo "════════ full-map 完整性对账 ════════"
printf '① HTTP 覆盖: %s / %s\n' "$http_registered" "$http_total"
printf '② WS  覆盖: %s / %s\n' "$ws_handled" "$ws_total"
printf '③ UC  真e2e验证: %s / %s（台账 %s）\n' "$uc_verified" "$uc_total" "$LEDGER"

fail=0
[ "${http_registered:-0}" -ge "${http_total:-99}" ] 2>/dev/null || { ylw "  → HTTP 未满（续补 outbound 命令）"; fail=1; }
[ "${ws_handled:-0}" -ge "$ws_total" ] 2>/dev/null || { ylw "  → WS 未满（续补 handler）"; fail=1; }
[ "${uc_verified:-0}" -ge "$uc_total" ] 2>/dev/null || { ylw "  → UC 真 e2e 未满（续跑 UC e2e + 更新台账）"; fail=1; }

if [ "$fail" -eq 0 ]; then
  grn "✅ 三表 100% 覆盖 —— 可判 DONE（仍需人工抽检生产）"
  exit 0
fi
red "🔴 未 100% —— workflow 接力续（resumeFromRunId），直到三表全绿"
exit 1
