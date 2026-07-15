#!/usr/bin/env bash
# Static drift/security gate for Helix trace instrumentation.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

FORBIDDEN_CORE_DEPS='opentelemetry|tracing-opentelemetry|reqwest|tokio|tauri|web-sys'
TRACE_LEAK_RE='traceparent|baggage|__trace|otel|opentelemetry'
TRACE_FILE_RE='TraceCarrier|TraceHooks|opentelemetry|traceparent|baggage|__trace'
# `baggage` 本身是被审计的数据词，不是输出动作；把它当 context 会把测试 fixture
# 中的 `{"text":...,"baggage":...}` 误判为日志。输出动作仍由 tracing/log/record 等覆盖。
TRACE_ATTR_CONTEXT='set_attribute|record\(|\.record|event!|span!|attribute::|\.attribute|tracing::|log::|println!|eprintln!|debug!|info!|warn!|error!'
SENSITIVE_WORDS='Authorization|cookie|cookieId|token|message|text|payload|body'
SENSITIVE_ATTR_RE="(${TRACE_ATTR_CONTEXT}).*(${SENSITIVE_WORDS})|(${SENSITIVE_WORDS}).*(${TRACE_ATTR_CONTEXT})"
RAW_TRACE_OUTPUT_RE="(${TRACE_ATTR_CONTEXT}).*raw_json|raw_json.*(${TRACE_ATTR_CONTEXT})"

echo "▶ trace-static [1/3] helix-core dependency tree has no OTel/runtime crates"
CORE_DEP_HITS="$(cargo tree -p helix-core | rg -n -i "$FORBIDDEN_CORE_DEPS" || true)"
if [ -n "$CORE_DEP_HITS" ]; then
  echo "❌ FAIL: helix-core dependency tree contains forbidden OTel/runtime crates"
  printf '%s\n' "$CORE_DEP_HITS"
  exit 1
fi
echo "✅ PASS: helix-core dependency tree is OTel/runtime-clean"

echo "▶ trace-static [2/3] trace sidecar terms do not leak into helix-core or helix-im/src"
TRACE_LEAK_HITS="$(
  rg -n -i "$TRACE_LEAK_RE" crates/helix-core crates/helix-im/src \
    --glob '!target/**' \
    --glob '!**/*.png' \
    --glob '!**/*.jpg' \
    --glob '!**/*.jpeg' \
    --glob '!**/*.gif' \
    --glob '!**/*.bin' || true
)"
if [ -n "$TRACE_LEAK_HITS" ]; then
  echo "❌ FAIL: trace sidecar terms leaked into helix-core or helix-im/src"
  printf '%s\n' "$TRACE_LEAK_HITS"
  exit 1
fi
echo "✅ PASS: no trace sidecar terms in helix-core or helix-im/src"

echo "▶ trace-static [3/3] trace files do not write sensitive/business payload into span/log attributes"
TRACE_FILES="$(mktemp "${TMPDIR:-/tmp}/helix-trace-files.XXXXXX")"
trap 'rm -f "$TRACE_FILES"' EXIT

rg -l "$TRACE_FILE_RE" \
  --glob '!target/**' \
  --glob '!docs/**' \
  --glob '!SESSION.md' \
  --glob '!scripts/trace-static-gate.sh' \
  --glob '!**/AGENTS.md' \
  --glob '!**/*.h' \
  --glob '!**/*.hpp' \
  --glob '!**/*.md' \
  --glob '!**/*.json' \
  --glob '!**/*.lock' \
  --glob '!**/*.png' \
  --glob '!**/*.jpg' \
  --glob '!**/*.jpeg' \
  --glob '!**/*.gif' \
  --glob '!**/*.bin' \
  >"$TRACE_FILES" || true

if [ ! -s "$TRACE_FILES" ]; then
  echo "✅ PASS: no trace-related source files found for sensitive attribute scan"
  exit 0
fi

echo "  trace-related source files scanned:"
sed 's/^/  - /' "$TRACE_FILES"

SENSITIVE_HITS="$(xargs rg -n -i "$SENSITIVE_ATTR_RE" <"$TRACE_FILES" || true)"
RAW_TRACE_HITS="$(xargs rg -n -i "$RAW_TRACE_OUTPUT_RE" <"$TRACE_FILES" || true)"

if [ -n "$SENSITIVE_HITS" ] || [ -n "$RAW_TRACE_HITS" ]; then
  echo "❌ FAIL: trace-related files appear to record sensitive/business payload into span/log attributes"
  if [ -n "$SENSITIVE_HITS" ]; then
    echo "  sensitive attribute/log context hits:"
    printf '%s\n' "$SENSITIVE_HITS"
  fi
  if [ -n "$RAW_TRACE_HITS" ]; then
    echo "  raw trace JSON output hits:"
    printf '%s\n' "$RAW_TRACE_HITS"
  fi
  exit 1
fi

echo "✅ PASS: trace-related files do not log sensitive/business payload attributes"
