#!/usr/bin/env bash
set -euo pipefail

DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$DIR/../.." && pwd)
DASHBOARD="$DIR/helix-runtime-performance-pre.json"
CONTRACT="$DIR/dashboard.contract.json"
GRAFANA_URL=${HELIX_GRAFANA_URL:-http://192.168.6.66:30300}
PROMETHEUS_URL=${HELIX_PRE_PROMETHEUS_URL:-http://192.168.6.66:30090}
DATASOURCE_UID=prometheus
FOLDER_UID=helix
CREDENTIAL_FILE=${HELIX_GRAFANA_CREDENTIAL_FILE:-"$ROOT/.perf/credentials/grafana.env"}

die() {
  printf 'GRAFANA_CONTRACT_ERROR: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "缺少命令: $1"
}

load_credentials() {
  if [[ -f "$CREDENTIAL_FILE" ]]; then
    # shellcheck disable=SC1090
    source "$CREDENTIAL_FILE"
  fi
  if [[ -n "${GRAFANA_API_TOKEN:-}" ]]; then
    AUTH_MODE=token
    return
  fi
  GRAFANA_USER=${GRAFANA_USER:-admin}
  if [[ -z "${GRAFANA_PASSWORD:-}" ]] && command -v security >/dev/null 2>&1; then
    GRAFANA_PASSWORD=$(security find-generic-password -s helix-grafana-pre -a "$GRAFANA_USER" -w 2>/dev/null || true)
  fi
  [[ -n "${GRAFANA_PASSWORD:-}" ]] || die "缺少 Grafana 凭据；请使用 Keychain、环境变量或 gitignored 本地凭据文件"
  AUTH_MODE=basic
}

api() {
  local method=$1 path=$2 data=${3:-}
  local args=(--fail --silent --show-error -X "$method" -H 'Accept: application/json')
  if [[ "$AUTH_MODE" == token ]]; then
    args+=(-H "Authorization: Bearer $GRAFANA_API_TOKEN")
  else
    args+=(-u "$GRAFANA_USER:$GRAFANA_PASSWORD")
  fi
  if [[ -n "$data" ]]; then
    args+=(-H 'Content-Type: application/json' --data-binary "$data")
  fi
  curl "${args[@]}" "$GRAFANA_URL$path"
}

generate() {
  require_cmd node
  node "$DIR/generate-dashboard.mjs"
}

contract() {
  require_cmd node
  node "$DIR/contract-check.mjs"
}

query_digest() {
  jq -r '.. | .expr? // empty' | LC_ALL=C sort | shasum -a 256 | awk '{print $1}'
}

verify_response() {
  local response=$1 local_digest remote_digest
  jq -e --arg uid "$(jq -r .uid "$CONTRACT")" --arg title "$(jq -r .title "$CONTRACT")" \
    '.dashboard.uid == $uid and .dashboard.title == $title' <<<"$response" >/dev/null \
    || die "Grafana 回读 UID/title 不匹配"
  jq -e --arg datasource "$DATASOURCE_UID" \
    '[.dashboard.panels[] | select(.type != "row") | .datasource.uid] | all(. == $datasource)' \
    <<<"$response" >/dev/null || die "Grafana 回读 datasource UID 不匹配"
  jq -e --slurpfile contract "$CONTRACT" \
    '([.dashboard.templating.list[].name] | sort) == ([$contract[0].variables[]] | sort)' \
    <<<"$response" >/dev/null || die "Grafana 回读变量集合不匹配"
  local_digest=$(query_digest <"$DASHBOARD")
  remote_digest=$(query_digest <<<"$response")
  [[ "$local_digest" == "$remote_digest" ]] || die "Grafana query digest 不匹配"
  printf 'dashboard verify PASS: uid=%s version=%s panels=%s queryDigest=%s\n' \
    "$(jq -r '.dashboard.uid' <<<"$response")" \
    "$(jq -r '.dashboard.version' <<<"$response")" \
    "$(jq '[.dashboard.panels[] | select(.type != "row")] | length' <<<"$response")" \
    "$remote_digest"
}

verify() {
  require_cmd curl
  require_cmd jq
  load_credentials
  local response
  response=$(api GET "/api/dashboards/uid/$(jq -r .uid "$CONTRACT")") \
    || die "Dashboard GET 回读失败"
  verify_response "$response"
}

query_check() {
  require_cmd curl
  require_cmd jq
  local checked=0 failed=0 query response
  while IFS= read -r query; do
    query=${query//\$service/.\*}
    query=${query//\$platform/.\*}
    query=${query//\$scenario/.\*}
    query=${query//\$channel_shape/.\*}
    query=${query//\$stage/.\*}
    query=${query//\$operation/.\*}
    query=${query//\$rate_window/5m}
    query=${query//\$__range/30m}
    response=$(curl --fail --silent --show-error --get \
      --data-urlencode "query=$query" "$PROMETHEUS_URL/api/v1/query") || response='{}'
    if ! jq -e '.status == "success"' <<<"$response" >/dev/null 2>&1; then
      failed=$((failed + 1))
    fi
    checked=$((checked + 1))
  done < <(jq -r '[.. | .expr? // empty] | unique[]' "$DASHBOARD")
  [[ "$failed" -eq 0 ]] || die "Prometheus query inspector: $failed/$checked 条查询失败"
  printf 'dashboard query-check PASS: Prometheus 接受 %s 条去重查询\n' "$checked"
}

apply() {
  require_cmd curl
  require_cmd jq
  contract
  load_credentials

  local datasource folder payload response
  datasource=$(api GET "/api/datasources/uid/$DATASOURCE_UID") || die "datasource API 不可用"
  jq -e --arg uid "$DATASOURCE_UID" '.uid == $uid' <<<"$datasource" >/dev/null \
    || die "datasource UID=$DATASOURCE_UID 不存在"

  folder=$(api GET "/api/folders/$FOLDER_UID" 2>/dev/null || true)
  if [[ -z "$folder" ]]; then
    folder=$(api POST '/api/folders' "$(jq -cn --arg uid "$FOLDER_UID" --arg title 'Helix' '{uid:$uid,title:$title}')") \
      || die "创建 Helix folder 失败"
  fi
  jq -e --arg uid "$FOLDER_UID" '.uid == $uid' <<<"$folder" >/dev/null \
    || die "Helix folder UID 不匹配"

  payload=$(jq -cn --slurpfile dashboard "$DASHBOARD" --arg folderUid "$FOLDER_UID" \
    '{dashboard:$dashboard[0],folderUid:$folderUid,overwrite:true,message:"Helix P0 dashboard contract apply"}')
  response=$(api POST '/api/dashboards/db' "$payload") || die "Dashboard apply 失败"
  jq -e --arg uid "$(jq -r .uid "$CONTRACT")" '.uid == $uid and (.status == "success")' \
    <<<"$response" >/dev/null || die "Dashboard apply 响应未确认成功"
  printf 'dashboard apply PASS: uid=%s version=%s status=%s\n' \
    "$(jq -r .uid <<<"$response")" "$(jq -r .version <<<"$response")" "$(jq -r .status <<<"$response")"
  verify
  query_check
}

usage() {
  printf '用法: %s {generate|contract|query-check|apply|verify}\n' "$0"
}

case "${1:-}" in
  generate) generate ;;
  contract) contract ;;
  query-check) query_check ;;
  apply) apply ;;
  verify) verify ;;
  *) usage; exit 2 ;;
esac
