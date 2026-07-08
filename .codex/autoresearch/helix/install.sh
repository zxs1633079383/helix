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
[ -f "$BUNDLE/agents/${TOPIC}-researcher.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-researcher.md" "$DEST/agents/${TOPIC}-researcher.md"
[ -f "$BUNDLE/agents/${TOPIC}-dev.md" ] && \
  cp "$BUNDLE/agents/${TOPIC}-dev.md" "$DEST/agents/${TOPIC}-dev.md"

echo "Installed ${TOPIC} into $DEST (copied)"
echo "Bundle remains at: $BUNDLE"
