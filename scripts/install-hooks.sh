#!/usr/bin/env bash
# 安装 git hooks：pre-push 跑 scope-aware 快速门；完整门由 GitLab CI/合并前执行。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
mkdir -p .git/hooks
cat > .git/hooks/pre-push <<'HOOK'
#!/usr/bin/env bash
exec scripts/gate-fast.sh
HOOK
chmod +x .git/hooks/pre-push scripts/gate.sh scripts/gate-fast.sh
echo "✅ pre-push hook 已安装（push 前自动跑 scripts/gate-fast.sh）"
