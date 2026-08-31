#!/usr/bin/env bash
set -euo pipefail
BUNDLE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOPIC="helix"
DEST="${CODEX_HOME:-$HOME/.codex}"
mkdir -p "$DEST/skills" "$DEST/agents"

if [ -d "$BUNDLE/skills/${TOPIC}-knowledge" ]; then
  rm -rf "$DEST/skills/${TOPIC}-knowledge"
  cp -R "$BUNDLE/skills/${TOPIC}-knowledge" "$DEST/skills/${TOPIC}-knowledge"
fi
if [ -d "$BUNDLE/skills/otel-prometheus-metrics" ]; then
  rm -rf "$DEST/skills/otel-prometheus-metrics"
  cp -R "$BUNDLE/skills/otel-prometheus-metrics" "$DEST/skills/otel-prometheus-metrics"
fi
[ -f "$BUNDLE/agents/${TOPIC}-researcher.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-researcher.md" "$DEST/agents/${TOPIC}-researcher.md"
[ -f "$BUNDLE/agents/${TOPIC}-dev.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-dev.md" "$DEST/agents/${TOPIC}-dev.md"
[ -f "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.md" ] && \
  cp "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.md" "$DEST/agents/helix-otel-prometheus-metrics-agent.md"

echo "Installed ${TOPIC} into $DEST (copied)"
echo "Bundle remains at: $BUNDLE"
