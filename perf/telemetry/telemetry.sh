#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
STATE_DIR=${HELIX_PERF_TELEMETRY_STATE_DIR:-"$ROOT/.perf/telemetry"}
SOURCE_OTEL_DIR="$ROOT/docker/otel"
RUNTIME_OTEL_DIR=${HELIX_PERF_OTEL_RUNTIME_DIR:-"$HOME/.helix-perf/otel"}
COMPOSE_FILE="$RUNTIME_OTEL_DIR/docker-compose.yaml"
COMPOSE_PROJECT=helix-perf-telemetry
EXPECTED_CONTEXT=${HELIX_PRE_KUBE_CONTEXT:-kubernetes-admin@kubernetes}
NAMESPACE=${HELIX_PRE_NAMESPACE:-monitoring}
COLLECTOR_SERVICE=${HELIX_PRE_COLLECTOR_SERVICE:-opentelemetry-collector}
PROMETHEUS_URL=${HELIX_PRE_PROMETHEUS_URL:-http://192.168.6.66:30090}
LOCAL_OTLP_GRPC_PORT=${HELIX_LOCAL_OTLP_GRPC_PORT:-4317}
LOCAL_OTLP_HTTP_PORT=${HELIX_LOCAL_OTLP_HTTP_PORT:-4318}
LOCAL_PROM_PORT=${HELIX_LOCAL_PROM_PORT:-9464}
TUNNEL_PORT=${HELIX_PRE_OTLP_TUNNEL_PORT:-14317}

die() {
  printf 'PERF_PRE_TELEMETRY_UNAVAILABLE: %s\n' "$*" >&2
  exit 1
}

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "缺少命令: $1"
}

port_is_listening() {
  nc -z 127.0.0.1 "$1" >/dev/null 2>&1
}

assert_port_free() {
  if port_is_listening "$1"; then
    die "本机端口 $1 已被占用，拒绝接管"
  fi
}

prom_query() {
  local query=$1
  curl --fail --silent --show-error --get \
    --data-urlencode "query=$query" \
    "$PROMETHEUS_URL/api/v1/query"
}

preflight() {
  require_cmd kubectl
  require_cmd docker
  require_cmd curl
  require_cmd jq
  require_cmd nc

  local context
  context=$(kubectl config current-context)
  [[ "$context" == "$EXPECTED_CONTEXT" ]] || die "kubectl context=$context，期望 $EXPECTED_CONTEXT"

  kubectl -n "$NAMESPACE" get deployment "$COLLECTOR_SERVICE" \
    -o jsonpath='{.status.readyReplicas}' | grep -Eq '^[1-9][0-9]*$' \
    || die "pre Collector Deployment 未 Ready"
  kubectl -n "$NAMESPACE" get service "$COLLECTOR_SERVICE" >/dev/null \
    || die "pre Collector Service 不存在"
  kubectl -n "$NAMESPACE" get servicemonitor "$COLLECTOR_SERVICE" >/dev/null \
    || die "pre Collector ServiceMonitor 不存在"

  local response
  response=$(prom_query 'up{namespace="monitoring",service="opentelemetry-collector"} == 1') \
    || die "Prometheus API 不可达"
  jq -e '.status == "success" and (.data.result | length > 0)' <<<"$response" >/dev/null \
    || die "Prometheus 中 pre Collector up!=1"
  printf 'telemetry preflight PASS: context、Collector、ServiceMonitor、Prometheus 均可用\n'
}

write_state() {
  local pf_pid=$1
  mkdir -p "$STATE_DIR"
  printf '%s\n' "$pf_pid" >"$STATE_DIR/port-forward.pid"
  ps -p "$pf_pid" -o lstart= | sed 's/^ *//' >"$STATE_DIR/port-forward.started"
  printf '%s\n' "$COMPOSE_PROJECT" >"$STATE_DIR/compose-project"
  docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" ps -q >"$STATE_DIR/container.ids" 2>/dev/null || true
  date -u +%Y-%m-%dT%H:%M:%SZ >"$STATE_DIR/started-at"
}

launchd_job_pid() {
  local label=$1
  launchctl print "gui/$(id -u)/$label" 2>/dev/null \
    | awk '/^[[:space:]]*pid = [0-9]+/ { print $3; exit }'
}

start_port_forward() {
  local log_file=$1
  if [[ "$(uname -s)" == "Darwin" ]] && command -v launchctl >/dev/null 2>&1; then
    local label="com.helix.perf.telemetry.$(id -u).$$"
    launchctl submit -l "$label" -o "$log_file" -e "$log_file" -- \
      "$(command -v kubectl)" -n "$NAMESPACE" port-forward --address=0.0.0.0 \
      "svc/$COLLECTOR_SERVICE" "$TUNNEL_PORT:4317"
    printf '%s\n' "$label" >"$STATE_DIR/port-forward.launchd-label"
    for _ in $(seq 1 30); do
      local pid
      pid=$(launchd_job_pid "$label")
      if [[ -n "$pid" ]]; then
        printf '%s\n' "$pid"
        return
      fi
      sleep 0.1
    done
    launchctl remove "$label" 2>/dev/null || true
    die "launchd port-forward 未在预算内产生 PID"
  fi

  nohup kubectl -n "$NAMESPACE" port-forward --address=0.0.0.0 \
    "svc/$COLLECTOR_SERVICE" "$TUNNEL_PORT:4317" </dev/null >"$log_file" 2>&1 &
  printf '%s\n' "$!"
}

prepare_runtime_files() {
  rm -rf "$RUNTIME_OTEL_DIR"
  mkdir -p "$RUNTIME_OTEL_DIR"
  cp "$SOURCE_OTEL_DIR/docker-compose.yaml" "$SOURCE_OTEL_DIR/otel-collector.yaml" \
    "$RUNTIME_OTEL_DIR/"
}

up() {
  [[ ! -e "$STATE_DIR/port-forward.pid" ]] || die "检测到已有 run-owned 状态；请先执行 down"
  preflight
  assert_port_free "$TUNNEL_PORT"
  assert_port_free "$LOCAL_OTLP_GRPC_PORT"
  assert_port_free "$LOCAL_OTLP_HTTP_PORT"
  assert_port_free "$LOCAL_PROM_PORT"
  prepare_runtime_files

  mkdir -p "$STATE_DIR"
  local pf_pid
  pf_pid=$(start_port_forward "$STATE_DIR/port-forward.log")
  for _ in $(seq 1 30); do
    port_is_listening "$TUNNEL_PORT" && break
    kill -0 "$pf_pid" 2>/dev/null || die "kubectl port-forward 提前退出"
    sleep 0.2
  done
  if ! port_is_listening "$TUNNEL_PORT"; then
    kill "$pf_pid" 2>/dev/null || true
    die "kubectl port-forward 未在预算内就绪"
  fi

  write_state "$pf_pid"
  local up_complete=0
  cleanup_failed_up() {
    local status=$?
    if [[ "$up_complete" -eq 0 ]]; then
      down >/dev/null 2>&1 || true
    fi
    return "$status"
  }
  trap cleanup_failed_up EXIT

  if ! docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" up -d; then
    die "本地 Collector 启动失败"
  fi
  write_state "$pf_pid"
  verify_runtime
  up_complete=1
  trap - EXIT
  printf 'telemetry up PASS: 本地 OTLP -> pre Collector 通道已就绪\n'
}

verify_owned_port_forward() {
  local pid
  [[ -f "$STATE_DIR/port-forward.pid" ]] || die "缺少 run-owned port-forward 状态"
  pid=$(cat "$STATE_DIR/port-forward.pid")
  if [[ -f "$STATE_DIR/port-forward.launchd-label" ]]; then
    local label current_pid
    label=$(cat "$STATE_DIR/port-forward.launchd-label")
    current_pid=$(launchd_job_pid "$label")
    [[ -n "$current_pid" && "$current_pid" == "$pid" ]] \
      || die "run-owned launchd port-forward 已退出或 PID 漂移"
  fi
  kill -0 "$pid" 2>/dev/null || die "run-owned port-forward 已退出"
  local command
  command=$(ps -p "$pid" -o command=)
  [[ "$command" == *"port-forward"*"svc/$COLLECTOR_SERVICE"*"$TUNNEL_PORT:4317"* ]] \
    || die "PID $pid 不再属于本次 port-forward，拒绝继续"
}

verify_runtime() {
  verify_owned_port_forward
  port_is_listening "$TUNNEL_PORT" || die "pre Collector tunnel 端口未监听"
  port_is_listening "$LOCAL_OTLP_GRPC_PORT" || die "本地 OTLP gRPC 端口未监听"
  port_is_listening "$LOCAL_OTLP_HTTP_PORT" || die "本地 OTLP HTTP 端口未监听"
  port_is_listening "$LOCAL_PROM_PORT" || die "本地 Prometheus exporter 端口未监听"

  local running
  running=$(docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" ps \
    --status running --services | sort)
  grep -qx 'otel-collector' <<<"$running" || die "本地 Collector 未运行"
}

emit_canary() {
  verify_runtime
  local now_ns start_ns body
  now_ns=$(($(date +%s) * 1000000000))
  start_ns=$((now_ns - 1000000000))
  body=$(printf '{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"helix-perf-canary"}},{"key":"deployment.environment","value":{"stringValue":"pre"}},{"key":"platform","value":{"stringValue":"native-macos"}}]},"scopeMetrics":[{"scope":{"name":"helix-perf-canary"},"metrics":[{"name":"helix_telemetry_canary_total","unit":"1","sum":{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{"attributes":[{"key":"scenario","value":{"stringValue":"P0-CANARY"}},{"key":"stage","value":{"stringValue":"telemetry"}},{"key":"operation","value":{"stringValue":"emit_canary"}},{"key":"status","value":{"stringValue":"ok"}},{"key":"channel_shape","value":{"stringValue":"none"}}],"startTimeUnixNano":"%s","timeUnixNano":"%s","asInt":"1"}]}}]}]}]}' "$start_ns" "$now_ns")
  curl --fail --silent --show-error \
    -H 'Content-Type: application/json' \
    --data-binary "$body" \
    "http://127.0.0.1:$LOCAL_OTLP_HTTP_PORT/v1/metrics" >/dev/null
  date -u +%Y-%m-%dT%H:%M:%SZ >"$STATE_DIR/canary-emitted-at"
  printf 'telemetry canary 已发送\n'
}

verify_canary() {
  local response metadata deadline
  emit_canary
  deadline=$((SECONDS + 95))
  while ((SECONDS < deadline)); do
    response=$(prom_query 'otel_helix_telemetry_canary_total{service_name="helix-perf-canary"}') || true
    [[ -n "${response:-}" ]] || response='{}'
    if jq -e '.status == "success" and (.data.result | length > 0)' \
      <<<"$response" >/dev/null 2>&1; then
      metadata=$(curl --fail --silent --show-error \
        "$PROMETHEUS_URL/api/v1/metadata?metric=otel_helix_telemetry_canary_total") || true
      [[ -n "${metadata:-}" ]] || metadata='{}'
      jq -e '.status == "success" and (.data | has("otel_helix_telemetry_canary_total"))' \
        <<<"$metadata" >/dev/null 2>&1 \
        || die "canary 有样本但 metadata 缺失，拒绝仅按命名推断 PASS"
      printf 'telemetry verify PASS: canary 已进入 pre Prometheus\n'
      return
    fi
    sleep 5
  done
  die "95s 内未在 pre Prometheus 发现 canary"
}

down() {
  [[ -d "$STATE_DIR" ]] || {
    printf 'telemetry down: 无 run-owned 状态，无需清理\n'
    return
  }

  if [[ -f "$STATE_DIR/port-forward.launchd-label" ]]; then
    local label
    label=$(cat "$STATE_DIR/port-forward.launchd-label")
    launchctl remove "$label" 2>/dev/null || true
  elif [[ -f "$STATE_DIR/port-forward.pid" ]]; then
    local pid
    pid=$(cat "$STATE_DIR/port-forward.pid")
    if kill -0 "$pid" 2>/dev/null && [[ "$(ps -p "$pid" -o command=)" == \
      *"port-forward"*"svc/$COLLECTOR_SERVICE"*"$TUNNEL_PORT:4317"* ]]; then
      kill "$pid"
      wait "$pid" 2>/dev/null || true
    fi
  fi

  if [[ -f "$STATE_DIR/compose-project" ]] && \
    [[ "$(cat "$STATE_DIR/compose-project")" == "$COMPOSE_PROJECT" ]]; then
    docker compose -p "$COMPOSE_PROJECT" -f "$COMPOSE_FILE" down >/dev/null
  fi
  rm -rf "$STATE_DIR"
  rm -rf "$RUNTIME_OTEL_DIR"
  printf 'telemetry down PASS: 仅清理本次创建的 port-forward/container\n'
}

usage() {
  printf '用法: %s {preflight|up|emit-canary|verify|down}\n' "$0"
}

case "${1:-}" in
  preflight) preflight ;;
  up) up ;;
  emit-canary) emit_canary ;;
  verify) preflight; verify_canary ;;
  down) down ;;
  *) usage; exit 2 ;;
esac
