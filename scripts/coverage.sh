#!/usr/bin/env bash
# coverage.sh — helix-im 覆盖率度量（im-takeover P0 基线工具链）
#
# 工具：cargo-llvm-cov（精度高于 tarpaulin，支持分支覆盖）。
# 用法：
#   scripts/coverage.sh                    全量 summary（写终端 + docs/review/coverage-baseline.md 不动）
#   scripts/coverage.sh --baseline         重写 docs/review/coverage-baseline.md 基线快照
#   scripts/coverage.sh --phase <file.rs>  对单文件断言 100% 行+函数覆盖（Phase gate 用）
#   scripts/coverage.sh --html             生成 HTML 报告（target/llvm-cov/html）
#
# Phase gate 判据（03-测试策略.md §3）：本 Phase 新增文件 100% lines+functions。
# **不要求全仓 100%**——存量走 baseline 只增不减。
#
# 退出码：0=ok / 1=覆盖率未达标 / 2=cargo-llvm-cov 未安装
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  cat >&2 <<'EOF'
⛔ cargo-llvm-cov 未安装。安装：
   cargo install cargo-llvm-cov --locked
   rustup component add llvm-tools-preview
EOF
  exit 2
fi

BASELINE_DOC="docs/review/coverage-baseline.md"
# helix-im 纯逻辑 crate，零运行时依赖（HX-C001），--no-default-features 与 03-测试策略.md 对齐。
COV_ARGS=(--no-default-features -p helix-im)

case "${1:-}" in
  --baseline)
    echo "▶ 生成覆盖率基线 → $BASELINE_DOC"
    SUMMARY="$(cargo llvm-cov "${COV_ARGS[@]}" --summary-only 2>/dev/null)"
    TS="$(date '+%Y-%m-%d %H:%M:%S %z')"
    HEAD_SHA="$(git rev-parse --short HEAD)"
    {
      echo "# 覆盖率基线（im-takeover P0 锁定）"
      echo ""
      echo "> 工具 cargo-llvm-cov · crate helix-im · \`--no-default-features\`"
      echo "> 生成 \`scripts/coverage.sh --baseline\`；快照时刻 commit \`$HEAD_SHA\` @ $TS"
      echo ""
      echo "## 基线语义（只增不减）"
      echo ""
      echo "- 本文件 = sync-v2 ~90% baseline 锁定时的全仓 helix-im 覆盖率快照。"
      echo "- **后续 Phase 判据**：本 Phase 新增文件跑 100% lines+functions（\`scripts/coverage.sh --phase <file>\`）；"
      echo "  全仓总覆盖率**只增不减**，不得因新增未覆盖代码回退。"
      echo "- 度量命令：\`cargo llvm-cov --no-default-features -p helix-im --summary-only\`。"
      echo ""
      echo "## 快照（\`--summary-only\`）"
      echo ""
      echo '```'
      echo "$SUMMARY"
      echo '```'
    } > "$BASELINE_DOC"
    echo "✓ 已写 $BASELINE_DOC"
    echo "$SUMMARY"
    ;;
  --phase)
    FILE="${2:-}"
    if [ -z "$FILE" ]; then
      echo "用法: scripts/coverage.sh --phase <src 相对路径或文件名，如 acl/from_tick.rs>" >&2
      exit 2
    fi
    # llvm-cov summary 的 Filename 列是 crate src 内相对路径（如 acl/from_tick.rs），非全路径。
    # 取传入 FILE 的 src 内相对形态（去掉 crates/helix-im/src/ 前缀）做匹配键。
    MATCH_KEY="${FILE#crates/helix-im/src/}"
    echo "▶ Phase 文件覆盖率断言（100% lines+functions）：$MATCH_KEY"
    cargo llvm-cov "${COV_ARGS[@]}" --summary-only 2>/dev/null | tee /tmp/helix-cov-summary.txt
    LINE="$(grep -F "$MATCH_KEY" /tmp/helix-cov-summary.txt || true)"
    if [ -z "$LINE" ]; then
      echo "⛔ 未在覆盖率报告找到 $MATCH_KEY（确认路径 + 该文件已被测试触达）"
      exit 1
    fi
    echo "  目标文件行：$LINE"
    # llvm-cov summary 行格式含多组 Cover%；用 awk 找 Lines/Functions 100%。
    # 简化判据：该行若含任何非 100.00% 的覆盖列 → fail（人工 review HTML 精判）。
    if echo "$LINE" | grep -qE '(\b[0-9]{1,2}\.[0-9]{2}%|\b0\.00%)'; then
      echo "⛔ $FILE 覆盖率未达 100%（见上行）"
      exit 1
    fi
    echo "✓ $FILE 覆盖率达标"
    ;;
  --html)
    echo "▶ 生成 HTML 覆盖率报告 → target/llvm-cov/html/index.html"
    cargo llvm-cov "${COV_ARGS[@]}" --html
    ;;
  "")
    echo "▶ helix-im 覆盖率 summary"
    cargo llvm-cov "${COV_ARGS[@]}" --summary-only
    ;;
  *)
    echo "未知参数: $1（见脚本头注释用法）" >&2
    exit 2
    ;;
esac
