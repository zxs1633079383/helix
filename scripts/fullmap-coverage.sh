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
# 端点真源 = `build()` 返回的 `(endpoint_path, body)` 首元字符串字面量。rustfmt 短返回写单行
# `Ok(("ep", ...))`、长返回拆多行（`Ok((` 后 `"ep",` 独占下一行）；故抓 `Ok((` 当行**及紧邻下一行**
# （`grep -A1`），再抽含 `/` 段的 endpoint-shaped 字面量（API 路径）。两形态精确命中、不误收 body
# json 键（json 键不在 `Ok((` 上下文）。单段端点（users/groups）此处不计——它们均在排除集或非 helix 域。
http_registered=$(grep -rhA1 'Ok((' crates/helix-im/src/outbound/ 2>/dev/null \
  | grep -oE '"[a-z][a-zA-Z0-9/_-]+"' | grep -E '/' \
  | tr -d '"' | sort -u | grep -vE 'search|createUser|/status' | wc -l | tr -d ' ')
# full-map HTTP 全集（posts+channel+user-misc 三 partial 的真实端点）。
# partials 1-2 用反引号 `` `/api/cses/x` ``、partial 3 裸写 `/api/cses/x`（表格行）→ 统一抽
# `/api/cses/<path>` token、剥反引号、剥 `/api/cses/` 前缀归一成相对端点（与 http_registered 同形，
# 计数可比）；只取表格行（`^|`）避开正文散文里的 base-prefix 说明行。排除 search/status/health。
# ⚠️ 旧正则 `` `/(posts?|channel...)` `` 不含 /api/cses 前缀 → 匹配 0/1（计数 bug，F2 修）。
http_total=$(grep -hE '^\|' \
  "$PARTIALS"/1--backend-http-posts.md "$PARTIALS"/2--backend-http-channel-sync.md "$PARTIALS"/3--backend-http-user-misc.md 2>/dev/null \
  | grep -oE '/api/cses/[a-zA-Z0-9/_-]+' | sed -e 's/`//g' -e 's#^/api/cses/##' \
  | sort -u | grep -viE 'search|/status|/health' | wc -l | tr -d ' ')

# ── ② WS:helix 处理的 action vs full-map partial 5 的事件全集 ──
# 动态从 partial 5 的「WS 事件分类总表」抽 action 名（每行 `| N | `action` …` 取首个反引号 token），
# 去重后对每个 action 查 helix src 是否有对应字符串字面量处理 → handled/total。
# ⚠️ 旧版硬编码 18 个 action 正则（漏 post_pin/quit_company/update_channel_member_nickName，且
#    含已废弃的 add_followers/add_mentions/posted）+ ws_total=19 写死 → 计数失真（F2 修）。
# ws_total = partial 5 去重后的「distinct action 数」（spec 称 19 推送，但 # 1 与 # 19 同为 `post`
#   action〔首发 vs bot/agent 复用同 wire action〕→ 去重后 distinct action = 18，gate 按 action 集对账）。
P5="$PARTIALS/5--backend-ws-events.md"
ws_actions=$(grep -E '^\| [0-9]+ \|' "$P5" 2>/dev/null | sed -E 's/^\| [0-9]+ \| `([a-zA-Z_]+)`.*/\1/' | sort -u)
ws_total=$(printf '%s\n' "$ws_actions" | grep -c . | tr -d ' ')
ws_handled=0
while IFS= read -r a; do
  [ -z "$a" ] && continue
  grep -rqE "\"$a\"" crates/helix-im/src/ 2>/dev/null && ws_handled=$((ws_handled+1))
done < <(printf '%s\n' "$ws_actions")

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
