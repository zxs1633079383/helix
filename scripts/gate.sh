#!/usr/bin/env bash
# Helix 提交闸门链（Bun 对齐 3.5）—— ROADMAP §三「结构闸门」全自动化。
# 用法：scripts/gate.sh          （快速门：check/clippy/test/双 grep/索引）
#       scripts/gate.sh --full   （追加 release 压测三件套）
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "▶ [1/7] 三栖 cargo check"
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet
cargo check --target wasm32-unknown-unknown -p helix-core -p helix-driver-wasm --quiet

echo "▶ [2/7] clippy（deny-list 生效）"
cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet -- -D warnings 2>/dev/null \
  || cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet

echo "▶ [3/7] 测试（五 crate：core/im/driver-native/driver-host + driver-ffi 含 f5 parity/e2e）"
cargo test -p helix-core -p helix-im -p helix-driver-native -p helix-driver-host -p helix-driver-ffi --quiet

echo "▶ [4/7] 零运行时闸门（HX-C001）"
if cargo tree -p helix-core | grep -qiE 'tokio|sqlx|reqwest|web-sys|tauri'; then
  echo "❌ helix-core 依赖树出现运行时依赖"; exit 1
fi

echo "▶ [5/7] 零业务概念闸门"
# grep 零命中退出码为 1 是期望结果——用 || true 防 pipefail 误杀
HITS=$( (grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ || true) | (grep -v '//' || true) | wc -l | tr -d ' ')
[ "$HITS" -eq 0 ] || { echo "❌ helix-core 出现 IM 语义词 ($HITS 处)"; exit 1; }

echo "▶ [6/7] harness 索引不变量"
IDX=$(grep -c '^| HX-C' CLAUDE.md || true); FILES=$(ls docs/harness/HX-C*.md | wc -l | tr -d ' ')
[ "$IDX" -eq "$FILES" ] || { echo "❌ harness 索引漂移：CLAUDE.md $IDX 行 vs 文件 $FILES 个"; exit 1; }

echo "▶ [7/7] public-api 面闸门（纪要拍板 #2）"
if command -v cargo-public-api >/dev/null 2>&1; then
  mkdir -p docs/api-baseline
  cargo public-api -p helix-core --simplified > /tmp/helix-core-api.txt
  if [ -f docs/api-baseline/helix-core.txt ]; then
    diff -u docs/api-baseline/helix-core.txt /tmp/helix-core-api.txt \
      || { echo "❌ helix-core 公开 API 面变化——确认后更新基线"; exit 1; }
  else
    cp /tmp/helix-core-api.txt docs/api-baseline/helix-core.txt
    echo "  ✦ 首次生成 API 基线 docs/api-baseline/helix-core.txt（记得 commit）"
  fi
else
  echo "  （cargo-public-api 未安装，跳过——cargo install cargo-public-api 后自动生效）"
fi

if [ "${1:-}" = "--full" ]; then
  echo "▶ [full] 承压三件套（release）"
  cargo test --release -p helix-im --test stress_test --quiet -- --test-threads=1
  cargo test --release -p helix-driver-native --test stress_storage --test stress_engine --quiet -- --test-threads=1
  echo "▶ [full] ffi C ABI cpp 直连冒烟（真 C++ link cdylib · cpp→rust→cpp）"
  # ffi 线完整自验（含 C++ shim + Dart console 第6层）见 scripts/ffi-verify.sh（需仓外 demo）
  bash scripts/cpp-smoke.sh
fi

echo "✅ 全部闸门通过"
