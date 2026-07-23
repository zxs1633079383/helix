#!/usr/bin/env bash
# lib-config.sh — 从 helix.config.yaml 读取 connection.* 并 export 脚本侧所需变量。
#
# 用法：source "$(dirname "${BASH_SOURCE[0]}")/lib-config.sh"
#
# 导出（已设则不覆盖，可通过 env 覆盖调试）：
#   HELIX_CONFIG_PATH   — helix.config.yaml 绝对路径（传给 host-cli --config）
#   HELIX_WS_URL        — connection.ws_url
#   HELIX_API_BASE      — connection.api_base
#   HELIX_DEFAULT_API_BASE — connection.default_api_base（Java；空则保持空）
#   HELIX_GO_LOG        — connection.go_log（脚本 tailing 服务端日志用）
#   HELIX_GO_PORT       — connection.go_port（脚本 nc/lsof 就绪探针用）
#   HELIX_START_HINT    — connection.start_hint（端口不通时输出的启动提示）
#   HELIX_COOKIE_ID     — identity.cookie_id（默认联调账号；--cookie-id flag 覆盖）
#   HELIX_COMPANY_ID    — identity.company_id（默认联调公司 id；--company-id flag 覆盖）
#
# 前置：优先 yq v4；未安装时用内置 Python 轻量解析器读取本仓简单 YAML。

_HELIX_SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
_HELIX_REPO_ROOT="$(cd "$_HELIX_SCRIPTS_DIR/.." && pwd)"

# 配置文件路径（可被 HELIX_CONFIG_PATH env 覆盖以指向非默认路径）
HELIX_CONFIG_PATH="${HELIX_CONFIG_PATH:-$_HELIX_REPO_ROOT/helix.config.yaml}"

if [ ! -f "$HELIX_CONFIG_PATH" ]; then
  echo "⚠️  lib-config.sh: 找不到 helix.config.yaml ($HELIX_CONFIG_PATH)；脚本变量未设置" >&2
  return 0 2>/dev/null || exit 0
fi

# 内部读取辅助（静默，空/null 均返空字符串）
_hx_yq() {
  local val
  if command -v yq >/dev/null 2>&1; then
    val=$(yq "$1" "$HELIX_CONFIG_PATH" 2>/dev/null)
  else
    val=$(python3 - "$HELIX_CONFIG_PATH" "$1" <<'PY'
import sys
path, expr = sys.argv[1], sys.argv[2].lstrip(".")
parts = [p for p in expr.split(".") if p]
stack = []
values = {}
for raw in open(path, encoding="utf-8"):
    line = raw.split("#", 1)[0].rstrip()
    if not line.strip() or ":" not in line:
        continue
    indent = len(line) - len(line.lstrip(" "))
    key, val = line.strip().split(":", 1)
    while stack and stack[-1][0] >= indent:
        stack.pop()
    stack.append((indent, key.strip()))
    full = ".".join(k for _, k in stack)
    val = val.strip()
    if val:
        if (val.startswith('"') and val.endswith('"')) or (val.startswith("'") and val.endswith("'")):
            val = val[1:-1]
        values[full] = val
print(values.get(".".join(parts), ""))
PY
)
  fi
  [ "$val" = "null" ] && val=""
  printf '%s' "$val"
}

# ── connection 块 ────────────────────────────────────────
HELIX_WS_URL="${HELIX_WS_URL:-$(_hx_yq '.connection.ws_url')}"
HELIX_API_BASE="${HELIX_API_BASE:-$(_hx_yq '.connection.api_base')}"

_hx_default_api="$(_hx_yq '.connection.default_api_base')"
HELIX_DEFAULT_API_BASE="${HELIX_DEFAULT_API_BASE:-$_hx_default_api}"
unset _hx_default_api

HELIX_GO_LOG="${HELIX_GO_LOG:-$(_hx_yq '.connection.go_log')}"
HELIX_GO_PORT="${HELIX_GO_PORT:-$(_hx_yq '.connection.go_port')}"
HELIX_START_HINT="${HELIX_START_HINT:-$(_hx_yq '.connection.start_hint')}"

# ── identity 块（默认联调账号；flag 覆盖优先级更高）────────
HELIX_COOKIE_ID="${HELIX_COOKIE_ID:-$(_hx_yq '.identity.cookie_id')}"
HELIX_COMPANY_ID="${HELIX_COMPANY_ID:-$(_hx_yq '.identity.company_id')}"

export HELIX_CONFIG_PATH HELIX_WS_URL HELIX_API_BASE HELIX_DEFAULT_API_BASE
export HELIX_GO_LOG HELIX_GO_PORT HELIX_START_HINT
export HELIX_COOKIE_ID HELIX_COMPANY_ID

unset _HELIX_SCRIPTS_DIR _HELIX_REPO_ROOT
unset -f _hx_yq
