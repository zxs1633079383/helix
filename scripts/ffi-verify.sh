#!/usr/bin/env bash
# ffi-verify.sh — driver-ffi 线独立自验闸门（1–6 层，全部无需 Xcode）。
#
# 为什么仍可独立运行：driver-ffi 这条线（Flutter FFI 端）应能脱离 native/sync-v2 等其它线，
# 独立、自动化、无需 Xcode 地自验到「C++ shim + Dart console 全链」——这是并行开发
# 支柱④「端线在自己 worktree 自验」对 ffi 线的兑现，也是未来 CI 的 ffi job。
#
# 默认 gate 对齐：`scripts/gate.sh` 会调用 `scripts/ffi-verify.sh --no-demo`，把仓内 1–5
# 层纳入提交闸门。第 6 层依赖仓外 Flutter demo / Dart 环境，仍只在 `scripts/gate.sh --full`
# 或手动运行本脚本时触发；7–8 层需要完整 Xcode，不属于默认 gate。
#
# 覆盖（金字塔 1–6，7–8 = Flutter UI / Maestro 需完整 Xcode，本脚本不含）：
#   1 Rust 单测 + e2e（command→Emit→批回调 / pump lifecycle drain / batch wire）
#   2 clippy FFI 铁律（undocumented_unsafe_blocks deny + unsafe doc · HX-C006）
#   3 零运行时 gate（core 不被 ffi 污染 · HX-C001）
#   4 native↔ffi egress byte parity（HX-C007 roundtrip）
#   5 C ABI cpp 直连冒烟（真 C++ link cdylib + #include helix.hpp · cpp→rust→cpp）
#   6 C++ shim + Dart console 全链（dart:ffi→shim→cdylib→pump→Emit→batch→listener）
#
# 用法: scripts/ffi-verify.sh [--no-demo]
#   --no-demo : 只跑仓内 1–5 层（跳过仓外 Flutter demo；默认 gate / CI 仓内 job 用）
# 仓外 demo 路径可经 HELIX_FFI_DEMO_DIR 覆盖（默认 /workspace/flutterWorkspace/helix_ffi_demo）。
# 退出码: 0=全过 / 1=某层 FAIL（输出哪层）。
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"

DEMO_DIR="${HELIX_FFI_DEMO_DIR:-/workspace/flutterWorkspace/helix_ffi_demo}"
RUN_DEMO=1; [ "${1:-}" = "--no-demo" ] && RUN_DEMO=0
fail() { echo "❌ FFI-VERIFY FAIL @ $1"; exit 1; }

echo "▶ [1/6] Rust 单测 + e2e（command→Emit→批回调 / pump lifecycle drain / batch wire）"
cargo test -p helix-driver-ffi --quiet || fail "1 cargo test -p helix-driver-ffi"

echo "▶ [2/6] clippy FFI 铁律（undocumented_unsafe_blocks deny · 来自 ffi Cargo.toml [lints.clippy]）"
# 不加 `-- -D warnings`：那会把整条 path-dep 链（core/im/host）的 style warning 连带升 error，
# 超出 ffi 铁律范围（core 既有 needless_return 等是另一条线的 style 债，gate.sh 亦 fallback 容忍）。
# ffi 自身的 undocumented_unsafe_blocks=deny 是 crate 级 lint 配置，不依赖 -D warnings 即生效。
cargo clippy -p helix-driver-ffi --all-targets --quiet || fail "2 clippy -p helix-driver-ffi"

echo "▶ [3/6] 零运行时 gate（core 不被 ffi 污染 · HX-C001）"
# grep 命中 = core 依赖树混入运行时 = 污染 = FAIL（与 gate.sh 第4步同语义）
if cargo tree -p helix-core | grep -qiE 'tokio|sqlx|reqwest|web-sys|tauri'; then
  echo "  ↓ 污染项："; cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
  fail "3 helix-core 依赖树出现运行时依赖"
fi

echo "▶ [4/6] native↔ffi egress byte parity（HX-C007 roundtrip）"
cargo test -p helix-driver-ffi --test f5_egress_parity --quiet || fail "4 f5_egress_parity"

echo "▶ [5/6] C ABI cpp 直连冒烟（真 C++ link cdylib · cpp→rust→cpp）"
bash scripts/cpp-smoke.sh || fail "5 cpp-smoke"

if [ "$RUN_DEMO" = 1 ]; then
  if [ -d "$DEMO_DIR" ]; then
    echo "▶ [6/6] C++ shim + Dart console 全链（dart:ffi→shim→cdylib→listener）@ $DEMO_DIR"
    ( cd "$DEMO_DIR" && bash tool/build_native.sh && dart run bin/console_smoke.dart ) \
      || fail "6 console_smoke（$DEMO_DIR）"
  else
    echo "⚠ [6/6] 跳过：仓外 demo 不存在（$DEMO_DIR）——设 HELIX_FFI_DEMO_DIR 或 --no-demo"
  fi
else
  echo "⏭ [6/6] 跳过（--no-demo）"
fi

LAYERS=$([ "$RUN_DEMO" = 1 ] && echo "1–6" || echo "1–5")
echo "✅ FFI-VERIFY PASS（$LAYERS；7–8 Flutter UI/Maestro 需完整 Xcode，另跑 flutter test integration_test -d macos）"
