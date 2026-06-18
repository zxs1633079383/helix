#!/usr/bin/env bash
# helix C++ 直连冒烟自动化（cpp → rust → cpp）。
#
# 链路：cargo build cdylib + helix.hpp → cmake build C++ smoke（真 C++ link cdylib）→ ctest。
# 测的是 C ABI 边界从**真 C++** 端到端：建 handle(传 vtable) → helix_command(im_send_message)
# → 收 on_domain_events_batch 批回调含 im:post:sending → 干净 destroy（join pump）。
# 离线可跑（optimistic emit，不依赖真服务器）。CI 可直接调本脚本。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
SMOKE_DIR="crates/helix-driver-ffi/smoke"

echo "▶ [1/3] build cdylib + helix.hpp (cargo build -p helix-driver-ffi --release)"
cargo build -p helix-driver-ffi --release

echo "▶ [2/3] cmake configure + build C++ smoke"
cmake -S "$SMOKE_DIR" -B "$SMOKE_DIR/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "$SMOKE_DIR/build" >/dev/null

echo "▶ [3/3] ctest"
ctest --test-dir "$SMOKE_DIR/build" --output-on-failure

echo "✅ C++ 直连冒烟通过（cpp→rust→cpp 端到端）"
