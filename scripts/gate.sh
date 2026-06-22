#!/usr/bin/env bash
# Helix 提交闸门链（Bun 对齐 3.5）—— ROADMAP §三「结构闸门」全自动化。
# 用法：scripts/gate.sh          （默认交付门：PC native + Flutter FFI 仓内验证）
#       scripts/gate.sh --full   （追加 release 压测三件套 + FFI demo 层，若仓外 demo 存在）
#
# Gate alignment:
# - 默认门必须覆盖当前产品化目标：PC Tauri/native + Flutter FFI。
# - FFI 仓内 1–5 层由 `scripts/ffi-verify.sh --no-demo` 纳入默认门。
# - ADR-006: wasm/web 已归档，不作为默认交付 gate；需要历史 trait-shape 自检时手动跑：
#   `cargo check --target wasm32-unknown-unknown -p helix-core -p helix-driver-wasm`。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "▶ [1/9] whitespace 闸门（git diff --check + --cached --check）"
git diff --check
git diff --cached --check

echo "▶ [2/9] 双端 cargo check（PC native + Flutter FFI；wasm 归档见 ADR-006）"
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet

echo "▶ [3/9] clippy（deny-list 生效）"
cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet -- -D warnings 2>/dev/null \
  || cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet

echo "▶ [4/9] 测试（五 crate：core/im/driver-native/driver-host + driver-ffi 含 f5 parity/e2e）"
cargo test -p helix-core -p helix-im -p helix-driver-native -p helix-driver-host -p helix-driver-ffi --quiet

echo "▶ [5/9] 零运行时闸门（HX-C001）"
if cargo tree -p helix-core | grep -qiE 'tokio|sqlx|reqwest|web-sys|tauri'; then
  echo "❌ helix-core 依赖树出现运行时依赖"; exit 1
fi

echo "▶ [6/9] 零业务概念闸门"
# grep 零命中退出码为 1 是期望结果——用 || true 防 pipefail 误杀
HITS=$( (grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ || true) | (grep -v '//' || true) | wc -l | tr -d ' ')
[ "$HITS" -eq 0 ] || { echo "❌ helix-core 出现 IM 语义词 ($HITS 处)"; exit 1; }

echo "▶ [7/9] harness 索引不变量"
IDX=$(grep -c '^| HX-C' CLAUDE.md || true); FILES=$(ls docs/harness/HX-C*.md | wc -l | tr -d ' ')
[ "$IDX" -eq "$FILES" ] || { echo "❌ harness 索引漂移：CLAUDE.md $IDX 行 vs 文件 $FILES 个"; exit 1; }

echo "▶ [8/9] Flutter FFI 仓内自验（1–5 层，无 Xcode / 无仓外 demo）"
bash scripts/ffi-verify.sh --no-demo

echo "▶ [9/9] public-api 面闸门（纪要拍板 #2）"
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
  echo "▶ [full] FFI 完整自验（仓外 demo 存在时追加第6层 Dart console）"
  bash scripts/ffi-verify.sh
fi

echo "✅ 全部闸门通过"
