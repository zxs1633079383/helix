#!/usr/bin/env bash
set -euo pipefail
BUNDLE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOPIC="helix"
DEST="${CODEX_HOME:-$HOME/.codex}"
USER_SKILLS_DEST="${CODEX_USER_SKILLS_HOME:-$HOME/.agents}/skills"
mkdir -p "$DEST/skills" "$DEST/agents" "$USER_SKILLS_DEST"

if [ -d "$BUNDLE/skills/${TOPIC}-knowledge" ]; then
  rm -rf "$DEST/skills/${TOPIC}-knowledge"
  cp -R "$BUNDLE/skills/${TOPIC}-knowledge" "$DEST/skills/${TOPIC}-knowledge"
  rm -rf "$USER_SKILLS_DEST/${TOPIC}-knowledge"
  cp -R "$BUNDLE/skills/${TOPIC}-knowledge" "$USER_SKILLS_DEST/${TOPIC}-knowledge"
fi
if [ -d "$BUNDLE/skills/otel-prometheus-metrics" ]; then
  rm -rf "$DEST/skills/otel-prometheus-metrics"
  cp -R "$BUNDLE/skills/otel-prometheus-metrics" "$DEST/skills/otel-prometheus-metrics"
  rm -rf "$USER_SKILLS_DEST/otel-prometheus-metrics"
  cp -R "$BUNDLE/skills/otel-prometheus-metrics" "$USER_SKILLS_DEST/otel-prometheus-metrics"
fi
[ -f "$BUNDLE/agents/${TOPIC}-researcher.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-researcher.md" "$DEST/agents/${TOPIC}-researcher.md"
[ -f "$BUNDLE/agents/${TOPIC}-dev.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-dev.md" "$DEST/agents/${TOPIC}-dev.md"
if [ -f "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.toml" ]; then
  cp "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.toml" "$DEST/agents/helix-otel-prometheus-metrics-agent.toml"
elif [ -f "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.md" ]; then
  cp "$BUNDLE/agents/helix-otel-prometheus-metrics-agent.md" "$DEST/agents/helix-otel-prometheus-metrics-agent.md"
fi

echo "Installed ${TOPIC} into $DEST (copied)"
echo "Installed ${TOPIC} user skills into $USER_SKILLS_DEST (copied)"
echo "Bundle remains at: $BUNDLE"
