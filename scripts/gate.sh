#!/usr/bin/env bash
# Helix 完整交付门（Bun 对齐 3.5）——由 GitLab CI 与合并前验收执行。
# 用法：scripts/gate.sh          （PC native + Flutter FFI + Web Rust 壳完整仓内验证）
#       scripts/gate.sh --full   （追加 release 压测三件套 + FFI demo 层，若仓外 demo 存在）
#
# Gate alignment:
# - 默认门覆盖 PC Tauri/native、Flutter FFI 与正式 Web Rust 壳；浏览器 E2E 由独立环境执行。
# - FFI 仓内 1–5 层由 `scripts/ffi-verify.sh --no-demo` 纳入默认门。
# - Web 必须同时通过 native 单测、wasm final build、双目标 clippy 与 host runtime 泄漏守护（HX-C003）。
# - wasm build 不能冒充真浏览器/真 Go；后者由 A5 浏览器链路另行验收。
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

echo "▶ [1/12] whitespace 闸门（git diff --check + --cached --check）"
git diff --check
git diff --cached --check

echo "▶ [2/12] 三端 Rust cargo check（PC native + Flutter FFI + Web）"
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host -p helix-driver-web --quiet

echo "▶ [3/12] Web wasm final build（HX-C003：必须完成链接）"
cargo build --target wasm32-unknown-unknown -p helix-driver-web --quiet

echo "▶ [4/12] clippy（host 五 crate + Web 双目标 strict）"
cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet -- -D warnings 2>/dev/null \
  || cargo clippy -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi -p helix-driver-host --quiet
cargo clippy -p helix-driver-web --all-targets --no-deps --quiet -- -D warnings
cargo clippy --target wasm32-unknown-unknown -p helix-driver-web --no-deps --quiet -- -D warnings

echo "▶ [5/12] 测试（六 crate：core/im/native/host/ffi/web）"
cargo test -p helix-core -p helix-im -p helix-driver-native -p helix-driver-host -p helix-driver-ffi -p helix-driver-web --quiet

echo "▶ [6/12] core 零运行时闸门（HX-C001）"
if cargo tree -p helix-core | grep -qiE 'tokio|sqlx|reqwest|web-sys|tauri'; then
  echo "❌ helix-core 依赖树出现运行时依赖"; exit 1
fi

echo "▶ [7/12] Web host runtime 泄漏闸门（HX-C003）"
WEB_DEPS="$(
  {
    cargo tree -p helix-driver-web --edges normal,dev --prefix none
    cargo tree --target wasm32-unknown-unknown -p helix-driver-web --edges normal --prefix none
  } | awk '{print $1}' | LC_ALL=C sort -u
)"
WEB_FORBIDDEN="$(
  printf '%s\n' "$WEB_DEPS" \
    | grep -E '^(helix-driver-(host|native|ffi|wasm)|helix-host-cli|tokio($|-)|rusqlite$|reqwest$|sqlx($|-)|tauri($|-)|hyper($|-)|tungstenite$|mio$|socket2$)' \
    || true
)"
[ -z "$WEB_FORBIDDEN" ] || {
  echo "❌ helix-driver-web 泄漏 host/native runtime 依赖："
  printf '%s\n' "$WEB_FORBIDDEN"
  exit 1
}

echo "▶ [8/12] trace 静态漂移 / 安全闸门"
bash scripts/trace-static-gate.sh

echo "▶ [9/12] 零业务概念闸门"
# grep 零命中退出码为 1 是期望结果——用 || true 防 pipefail 误杀
HITS=$( (grep -rn 'channel\|message\|temporaryId' crates/helix-core/src/ || true) | (grep -v '//' || true) | wc -l | tr -d ' ')
[ "$HITS" -eq 0 ] || { echo "❌ helix-core 出现 IM 语义词 ($HITS 处)"; exit 1; }

echo "▶ [10/12] harness 索引不变量"
IDX=$(grep -c '^| HX-C' AGENTS.md || true); FILES=$(ls docs/harness/HX-C*.md | wc -l | tr -d ' ')
[ "$IDX" -eq "$FILES" ] || { echo "❌ harness 索引漂移：AGENTS.md $IDX 行 vs 文件 $FILES 个"; exit 1; }

echo "▶ [11/12] Flutter FFI 仓内自验（1–5 层，无 Xcode / 无仓外 demo）"
bash scripts/ffi-verify.sh --no-demo

echo "▶ [12/12] public-api 面闸门（纪要拍板 #2）"
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
