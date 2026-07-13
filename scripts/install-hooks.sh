#!/usr/bin/env bash
# 安装 git hooks：pre-push 跑闸门链（本地版 CI——仓库接远端后由 GHA 接管）
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
mkdir -p .git/hooks
cat > .git/hooks/pre-push <<'HOOK'
#!/usr/bin/env bash
exec scripts/gate.sh
HOOK
chmod +x .git/hooks/pre-push scripts/gate.sh
echo "✅ pre-push hook 已安装（push 前自动跑 scripts/gate.sh）"
