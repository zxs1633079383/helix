#!/usr/bin/env bash
# 本地 scope-aware 快速门。完整交付验收由 CI/合并前的 scripts/gate.sh 负责。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

BASE="${HELIX_GATE_BASE:-}"
DRY_RUN=0
PATHS=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --base)
      BASE="${2:?--base requires a git ref}"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    *)
      PATHS+=("$1")
      shift
      ;;
  esac
done

run_cmd() {
  printf '  +'
  printf ' %q' "$@"
  printf '\n'
  if [ "$DRY_RUN" -eq 0 ]; then
    "$@"
  fi
}

if [ "${#PATHS[@]}" -gt 0 ]; then
  CHANGED=$(printf '%s\n' "${PATHS[@]}" | sort -u)
else
  if [ -z "$BASE" ]; then
    if UPSTREAM=$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null); then
      BASE=$(git merge-base HEAD "$UPSTREAM")
    elif git show-ref --verify --quiet refs/remotes/origin/HEAD; then
      BASE=$(git merge-base HEAD origin/HEAD)
    elif git rev-parse --verify HEAD^ >/dev/null 2>&1; then
      BASE=HEAD^
    fi
  fi

  CHANGED=$({
    if [ -n "$BASE" ]; then
      git diff --name-only "$BASE"...HEAD
    fi
    git diff --name-only
    git diff --cached --name-only
    git ls-files --others --exclude-standard
  } | awk '!/^(\.codebase-memory|outputs)\//' | sort -u)
fi

matches() {
  printf '%s\n' "$CHANGED" | grep -Eq "$1"
}

echo "▶ [fast 1/2] whitespace"
if [ -n "$BASE" ]; then
  run_cmd git diff --check "$BASE"...HEAD
fi
run_cmd git diff --check
run_cmd git diff --cached --check

if [ -z "$CHANGED" ]; then
  echo "✅ fast gate：无待验证改动"
  exit 0
fi

echo "▶ [fast 2/2] scope-aware checks"

if matches '(^|/)(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml)$|^crates/helix-core/|^crates/helix-driver-|\.rs$'; then
  if ! matches '^crates/(helix-im|helix-host-cli)/' || matches '^crates/helix-core/|^crates/helix-driver-|(^|/)(Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml)$'; then
    echo "  高风险 Rust/Cargo 边界改动 → 升级完整交付门"
    run_cmd bash scripts/gate.sh
    if [ "$DRY_RUN" -eq 1 ]; then
      echo "✅ dry-run：已选择完整交付门路由"
    else
      echo "✅ fast gate：完整交付门通过"
    fi
    exit 0
  fi
fi

if matches '^scripts/.*\.sh$'; then
  run_cmd bash -n scripts/*.sh
fi

if matches '^\.agents/manifests/.*\.json$'; then
  for file in .agents/manifests/*.json; do
    run_cmd jq empty "$file"
  done
fi

if matches '^tools/web-w0-probe/.*\.m?js$'; then
  while IFS= read -r file; do
    if [ -f "$file" ]; then
      run_cmd node --check "$file"
    fi
  done < <(printf '%s\n' "$CHANGED" | grep -E '^tools/web-w0-probe/.*\.m?js$')
  if compgen -G 'tools/web-w0-probe/*.test.mjs' >/dev/null; then
    run_cmd node --test tools/web-w0-probe/*.test.mjs
  fi
fi

if matches '^crates/helix-im/'; then
  run_cmd cargo check -p helix-im --quiet
  run_cmd cargo test -p helix-im --quiet
fi

if matches '^crates/helix-host-cli/'; then
  run_cmd cargo check -p helix-host-cli --quiet
  run_cmd cargo test -p helix-host-cli --quiet
fi

if [ "$DRY_RUN" -eq 1 ]; then
  echo "✅ dry-run：路由选择完成"
else
  echo "✅ fast gate 通过；完整 scripts/gate.sh 留给 CI/合并前执行"
fi
