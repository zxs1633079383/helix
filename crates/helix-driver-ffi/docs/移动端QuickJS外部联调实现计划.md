# Mobile QuickJS External Integration Implementation Plan

> 历史说明：本文是 2026-07-03 旧外部联调计划快照，保留用于追溯最早的 C++/QuickJS
> 接入步骤。2026-07-06 后当前执行真源已收敛为 **QuickJS native `im.*` +
> C++ `MobileImSdk` / SDK hpp**：不得再按本文旧任务创建 `js/im_api.js`、
> `createImApi(core)` 或 JS 业务 facade。现行边界以
> [`移动端QuickJS最后一公里接入SPEC.md`](./移动端QuickJS最后一公里接入SPEC.md)、
> [`移动端Command-Query-Event契约SPEC.md`](./移动端Command-Query-Event契约SPEC.md)、
> [`多端联调测试指南.md`](./多端联调测试指南.md) 为准。

**Goal:** 在 `/workspace/c/mobile-qucik-c++` 中实现移动端最后一公里：C++ `CoreBridge` 直连真实 `helix-driver-ffi`，注册到 QuickJS，并通过 JS `im.send(...)` 驱动发送链路 smoke。

**Architecture:** `helix` 继续只提供 Rust FFI ABI、`helix.h` / `helix.hpp`、native library、command/query/event 契约与 Rust 验收；外部项目 `/workspace/c/mobile-qucik-c++` 承接 C++、QuickJS、JS facade、Android/iOS 打包与移动端 smoke。事件跨 FFI 边界必须批量交付，C++ 回调内立即拷贝，QuickJS 线程通过队列拉取事件。

**Tech Stack:** Rust `helix-driver-ffi` cdylib、C++17、QuickJS C API、POSIX shell、Makefile、命令行 smoke；Android/iOS/Flutter e2e 作为环境就绪后的后续验收层。

## Global Constraints

- helix 仓不得创建 C++ CoreBridge、QuickJS binding、Android/iOS 构建脚本或移动端编译产物。
- `/workspace/c/mobile-qucik-c++` 是移动端 C++ / QuickJS 集成项目根目录。
- `/workspace/c/quickjs` 是 QuickJS 源码仓，不是本次集成项目，不要往里面放本项目文档或实现。
- 外部项目必须保留 `docs/移动端QuickJS最后一公里接入SPEC.md` 的副本。
- C++ 必须链接真实 `helix-driver-ffi` 产物，最终 S0/S1/S2 不允许使用 `rust_core_mock.cpp`。
- FFI 事件批回调中的 `ptr,len` 只在回调期间有效，C++ 必须立即拷贝。
- QuickJS API 只允许在 QuickJS runtime 所属线程调用。
- `call("im_send", payload)` 只表示 Rust 已接收命令，不表示发送成功。
- 移动端 P0 不公开 `im:post:send-failed`，JS 不允许从 catch 或错误字符串合成失败事件。
- 本机 Android/iOS/Flutter e2e 环境缺失时，先完成 C++ 直连和 QuickJS smoke，并在验收文档记录环境缺口。

---

## File Structure

### helix 只维护文档和契约

- `crates/helix-driver-ffi/docs/移动端QuickJS最后一公里接入SPEC.md`
  最后一公里 SPEC 真源，记录边界、事件模型、验收分层。

- 移动端实现计划已迁移到 `/System/Volumes/Data/workspace/c/mobile-qucik-c++/docs/移动端QuickJS外部联调实现计划.md`；helix 仓只保留 FFI 契约与边界文档。
  本实现计划真源。

### `/workspace/c/mobile-qucik-c++` 外部项目新增

- `README.md`
  项目入口、依赖、快速 smoke 命令。

- `docs/移动端QuickJS最后一公里接入SPEC.md`
  从 helix 复制的 SPEC。

- `docs/多端联调运行手册.md`
  记录 helix 构建、C++ smoke、QuickJS smoke、未来 Android/iOS/Flutter e2e 的运行顺序和失败定位。

- `config/helix.env.example`
  本机路径与运行时 URL 模板。

- `scripts/build-helix-ffi.sh`
  在 helix 仓构建 `helix-driver-ffi`，复制 `helix.h`、`helix.hpp` 和 native library 到本项目 `third_party/helix/`。

- `scripts/build-quickjs.sh`
  使用 `/workspace/c/quickjs` 构建或定位 QuickJS 静态库与头文件。

- `scripts/run-cpp-smoke.sh`
  编译并运行 C++ 直连 smoke。

- `scripts/run-quickjs-smoke.sh`
  编译 QuickJS host 并执行 JS smoke。

- `Makefile`
  聚合 `helix-artifacts`、`quickjs-artifacts`、`cpp-smoke`、`quickjs-smoke`、`verify`。

- `cpp/HelixEventQueue.hpp`
  线程安全事件批队列，负责保存 FFI 回调中拷贝出来的 bytes。

- `cpp/CoreBridge.hpp`
  C++ 门面类声明：`version/capabilities/call/query/drainEvents/dispose`。

- `cpp/CoreBridge.cpp`
  C++ 门面实现，持有 `HelixHandle*`，调用 `helix_*` C ABI。

- `quickjs/bind_core_bridge.cpp`
  QuickJS native class 注册，把 C++ `CoreBridge` 暴露成 JS `CoreBridge`。

- `quickjs/quickjs_host.cpp`
  命令行 QuickJS smoke host，加载 binding 并执行 JS 文件。

- `js/core_bridge.js`
  JS 低层包装，处理 `drainEvents()` 和事件订阅。

- `js/im_api.js`
  业务 facade，暴露 `createImApi(core)` 和 `im.send(...)`。

- `smoke/cpp_core_bridge_smoke.cpp`
  C++ 直连真实 FFI smoke。

- `smoke/quickjs_core_bridge_smoke.js`
  QuickJS 低层 class smoke。

- `smoke/quickjs_im_send_smoke.js`
  JS `im.send(...)` facade smoke。

---

### Task 1: 外部项目骨架和 helix 产物导入

**Files:**
- Create: `/workspace/c/mobile-qucik-c++/README.md`
- Create: `/workspace/c/mobile-qucik-c++/config/helix.env.example`
- Create: `/workspace/c/mobile-qucik-c++/scripts/build-helix-ffi.sh`
- Create: `/workspace/c/mobile-qucik-c++/Makefile`
- Modify: `/workspace/c/mobile-qucik-c++/docs/移动端QuickJS最后一公里接入SPEC.md`

**Interfaces:**
- Consumes: helix repo at `/System/Volumes/Data/workspace/rust/helix`
- Produces:
  - `/workspace/c/mobile-qucik-c++/third_party/helix/include/helix.h`
  - `/workspace/c/mobile-qucik-c++/third_party/helix/include/helix.hpp`
  - `/workspace/c/mobile-qucik-c++/third_party/helix/lib/libhelix_driver_ffi.{dylib,so,a}`

- [ ] **Step 1: Write scaffold files**

Create `config/helix.env.example`:

```bash
HELIX_REPO=/System/Volumes/Data/workspace/rust/helix
HELIX_API_BASE_URL=http://127.0.0.1:8066
HELIX_WS_URL=ws://127.0.0.1:8066/api/cses/websocket
QUICKJS_ROOT=/workspace/c/quickjs
```

Create `scripts/build-helix-ffi.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HELIX_REPO="${HELIX_REPO:-/System/Volumes/Data/workspace/rust/helix}"
OUT="$ROOT/third_party/helix"

mkdir -p "$OUT/include" "$OUT/lib"

cd "$HELIX_REPO"
cargo build -p helix-driver-ffi --release

cp crates/helix-driver-ffi/helix.h "$OUT/include/helix.h"
cp crates/helix-driver-ffi/helix.hpp "$OUT/include/helix.hpp"

found=0
for lib in \
  target/release/libhelix_driver_ffi.dylib \
  target/release/libhelix_driver_ffi.so \
  target/release/libhelix_driver_ffi.a
do
  if [ -f "$lib" ]; then
    cp "$lib" "$OUT/lib/"
    found=1
  fi
done

if [ "$found" -ne 1 ]; then
  echo "missing helix-driver-ffi native library under $HELIX_REPO/target/release" >&2
  exit 1
fi

echo "helix artifacts copied to $OUT"
```

Create `Makefile`:

```make
SHELL := /usr/bin/env bash

.PHONY: helix-artifacts quickjs-artifacts cpp-smoke quickjs-smoke verify clean

helix-artifacts:
	./scripts/build-helix-ffi.sh

quickjs-artifacts:
	./scripts/build-quickjs.sh

cpp-smoke: helix-artifacts
	./scripts/run-cpp-smoke.sh

quickjs-smoke: helix-artifacts quickjs-artifacts
	./scripts/run-quickjs-smoke.sh

verify: cpp-smoke quickjs-smoke

clean:
	rm -rf build third_party/helix third_party/quickjs
```

Create `README.md`:

```markdown
# mobile-qucik-c++

移动端 QuickJS 最后一公里集成项目。

职责：

- 封装 helix `helix.h` / `helix.hpp` 为 C++ `CoreBridge`。
- 注册 QuickJS native class。
- 暴露 JS `createImApi(core)` 和 `im.send(...)`。
- 运行 C++ 直连真实 helix-driver-ffi smoke。
- 运行 QuickJS smoke。

快速验证：

```bash
cd /workspace/c/mobile-qucik-c++
make verify
```

路径约定：

- helix: `/System/Volumes/Data/workspace/rust/helix`
- QuickJS 源码: `/workspace/c/quickjs`
- 本项目: `/workspace/c/mobile-qucik-c++`
```

- [ ] **Step 2: Run artifact import and confirm failure mode is actionable**

Run:

```bash
cd /workspace/c/mobile-qucik-c++
chmod +x scripts/build-helix-ffi.sh
./scripts/build-helix-ffi.sh
```

Expected:

- Success creates `third_party/helix/include/helix.h`.
- Success creates `third_party/helix/include/helix.hpp`.
- Success creates one native library under `third_party/helix/lib/`.
- Failure prints a missing Cargo/library message, not a silent empty folder.

- [ ] **Step 3: Commit external scaffold**

```bash
cd /workspace/c/mobile-qucik-c++
git status --porcelain=v1 --untracked-files=all || true
```

If the external project is not a git repo, record that in `docs/多端联调运行手册.md` in Task 6. If it is a git repo:

```bash
git add README.md config/helix.env.example scripts/build-helix-ffi.sh Makefile docs/移动端QuickJS最后一公里接入SPEC.md
git commit -m "chore: 初始化 helix FFI 集成骨架"
```

---

### Task 2: C++ CoreBridge 和事件队列

**Files:**
- Create: `/workspace/c/mobile-qucik-c++/cpp/HelixEventQueue.hpp`
- Create: `/workspace/c/mobile-qucik-c++/cpp/CoreBridge.hpp`
- Create: `/workspace/c/mobile-qucik-c++/cpp/CoreBridge.cpp`

**Interfaces:**
- Consumes:
  - `third_party/helix/include/helix.hpp`
  - `helix_create_with_network`
  - `helix_destroy`
  - `helix_command`
  - `helix_query`
  - `helix_query_free`
- Produces:
  - `mobile::CoreBridge::version() -> std::string`
  - `mobile::CoreBridge::capabilities() -> std::string`
  - `mobile::CoreBridge::call(name, json) -> int32_t`
  - `mobile::CoreBridge::query(name, json) -> std::string`
  - `mobile::CoreBridge::drainEvents() -> std::vector<std::string>`
  - `mobile::CoreBridge::dispose() -> void`

- [ ] **Step 1: Write compile smoke before implementation**

Create `/workspace/c/mobile-qucik-c++/smoke/cpp_core_bridge_smoke.cpp` with only includes first:

```cpp
#include "CoreBridge.hpp"

int main() {
  mobile::CoreBridgeConfig config;
  config.api_base_url = "http://127.0.0.1:8066";
  config.ws_url = "ws://127.0.0.1:8066/api/cses/websocket";
  mobile::CoreBridge bridge(config);
  bridge.dispose();
  return 0;
}
```

Run:

```bash
cd /workspace/c/mobile-qucik-c++
c++ -std=c++17 -Icpp -Ithird_party/helix/include smoke/cpp_core_bridge_smoke.cpp -c -o /tmp/cpp_core_bridge_smoke.o
```

Expected: FAIL because `cpp/CoreBridge.hpp` does not exist.

- [ ] **Step 2: Implement event queue**

Create `cpp/HelixEventQueue.hpp`:

```cpp
#pragma once

#include <mutex>
#include <string>
#include <utility>
#include <vector>

namespace mobile {

class HelixEventQueue {
 public:
  void push(const uint8_t* ptr, size_t len) {
    if (ptr == nullptr || len == 0) {
      return;
    }
    std::lock_guard<std::mutex> lock(mu_);
    batches_.emplace_back(reinterpret_cast<const char*>(ptr), len);
  }

  std::vector<std::string> drain() {
    std::lock_guard<std::mutex> lock(mu_);
    std::vector<std::string> out;
    out.swap(batches_);
    return out;
  }

 private:
  std::mutex mu_;
  std::vector<std::string> batches_;
};

}  // namespace mobile
```

- [ ] **Step 3: Implement CoreBridge header**

Create `cpp/CoreBridge.hpp`:

```cpp
#pragma once

#include <cstdint>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

#include "HelixEventQueue.hpp"
#include "helix.hpp"

namespace mobile {

struct CoreBridgeConfig {
  std::string api_base_url = "http://127.0.0.1:8066";
  std::string ws_url = "ws://127.0.0.1:8066/api/cses/websocket";
};

class CoreBridge final {
 public:
  explicit CoreBridge(CoreBridgeConfig config);
  ~CoreBridge();

  CoreBridge(const CoreBridge&) = delete;
  CoreBridge& operator=(const CoreBridge&) = delete;

  std::string version();
  std::string capabilities();
  int32_t call(std::string_view name, std::string_view json_payload);
  std::string query(std::string_view name, std::string_view json_payload);
  std::vector<std::string> drainEvents();
  void dispose();

 private:
  static void onEvents(void* ctx, const uint8_t* ptr, uintptr_t len);
  static uint64_t nowMs(void* ctx);
  void ensureAlive() const;

  CoreBridgeConfig config_;
  HelixEventQueue events_;
  HelixHandle* handle_ = nullptr;
};

}  // namespace mobile
```

- [ ] **Step 4: Implement CoreBridge source**

Create `cpp/CoreBridge.cpp`:

```cpp
#include "CoreBridge.hpp"

#include <chrono>

namespace mobile {

namespace {
const uint8_t* bytes(std::string_view value) {
  return reinterpret_cast<const uint8_t*>(value.data());
}
}

CoreBridge::CoreBridge(CoreBridgeConfig config) : config_(std::move(config)) {
  CppHostVtable vtable{};
  vtable.on_domain_events_batch = &CoreBridge::onEvents;
  vtable.now_ms = &CoreBridge::nowMs;
  vtable.ctx = this;

  handle_ = helix_create_with_network(
      bytes(config_.api_base_url),
      config_.api_base_url.size(),
      bytes(config_.ws_url),
      config_.ws_url.size(),
      vtable);

  if (handle_ == nullptr) {
    throw std::runtime_error("helix_create_with_network returned null");
  }
}

CoreBridge::~CoreBridge() {
  dispose();
}

std::string CoreBridge::version() {
  return query("helix_version", "{}");
}

std::string CoreBridge::capabilities() {
  return query("helix_capabilities", "{}");
}

int32_t CoreBridge::call(std::string_view name, std::string_view json_payload) {
  ensureAlive();
  return helix_command(
      handle_,
      bytes(name),
      name.size(),
      bytes(json_payload),
      json_payload.size());
}

std::string CoreBridge::query(std::string_view name, std::string_view json_payload) {
  ensureAlive();
  uint8_t* out_ptr = nullptr;
  uintptr_t out_len = 0;
  int32_t rc = helix_query(
      handle_,
      bytes(name),
      name.size(),
      bytes(json_payload),
      json_payload.size(),
      &out_ptr,
      &out_len);
  if (rc != FFI_OK) {
    throw std::runtime_error("helix_query failed rc=" + std::to_string(rc));
  }
  std::string result(reinterpret_cast<const char*>(out_ptr), out_len);
  helix_query_free(out_ptr, out_len);
  return result;
}

std::vector<std::string> CoreBridge::drainEvents() {
  return events_.drain();
}

void CoreBridge::dispose() {
  if (handle_ != nullptr) {
    HelixHandle* owned = handle_;
    handle_ = nullptr;
    helix_destroy(owned);
  }
}

void CoreBridge::onEvents(void* ctx, const uint8_t* ptr, uintptr_t len) {
  auto* self = static_cast<CoreBridge*>(ctx);
  if (self != nullptr) {
    self->events_.push(ptr, static_cast<size_t>(len));
  }
}

uint64_t CoreBridge::nowMs(void*) {
  using namespace std::chrono;
  return static_cast<uint64_t>(
      duration_cast<milliseconds>(system_clock::now().time_since_epoch()).count());
}

void CoreBridge::ensureAlive() const {
  if (handle_ == nullptr) {
    throw std::runtime_error("CoreBridge disposed");
  }
}

}  // namespace mobile
```

- [ ] **Step 5: Run compile smoke**

```bash
cd /workspace/c/mobile-qucik-c++
c++ -std=c++17 \
  -Icpp \
  -Ithird_party/helix/include \
  smoke/cpp_core_bridge_smoke.cpp \
  cpp/CoreBridge.cpp \
  -Lthird_party/helix/lib \
  -lhelix_driver_ffi \
  -o build/cpp_core_bridge_smoke
```

Expected: PASS compile. Runtime linking may need `DYLD_LIBRARY_PATH` on macOS or `LD_LIBRARY_PATH` on Linux; Task 3 captures that in script.

- [ ] **Step 6: Commit CoreBridge**

```bash
cd /workspace/c/mobile-qucik-c++
git add cpp/HelixEventQueue.hpp cpp/CoreBridge.hpp cpp/CoreBridge.cpp smoke/cpp_core_bridge_smoke.cpp
git commit -m "feat: 封装 helix FFI CoreBridge"
```

If this directory is not a git repo, append the file list and verification result to `docs/多端联调运行手册.md`.

---

### Task 3: C++ 直连真实 helix-driver-ffi smoke

**Files:**
- Modify: `/workspace/c/mobile-qucik-c++/smoke/cpp_core_bridge_smoke.cpp`
- Create: `/workspace/c/mobile-qucik-c++/scripts/run-cpp-smoke.sh`

**Interfaces:**
- Consumes: `mobile::CoreBridge`
- Produces: S0 smoke proving real FFI direct link.

- [ ] **Step 1: Expand C++ smoke assertions**

Replace `smoke/cpp_core_bridge_smoke.cpp`:

```cpp
#include "CoreBridge.hpp"

#include <cstdlib>
#include <iostream>
#include <string>

namespace {

void require_contains(const std::string& haystack, const std::string& needle) {
  if (haystack.find(needle) == std::string::npos) {
    std::cerr << "missing: " << needle << "\nfrom: " << haystack << "\n";
    std::exit(1);
  }
}

}  // namespace

int main() {
  mobile::CoreBridgeConfig config;
  const char* api = std::getenv("HELIX_API_BASE_URL");
  const char* ws = std::getenv("HELIX_WS_URL");
  if (api != nullptr) {
    config.api_base_url = api;
  }
  if (ws != nullptr) {
    config.ws_url = ws;
  }

  mobile::CoreBridge bridge(config);

  std::string version = bridge.version();
  require_contains(version, "\"abi\":1");

  std::string caps = bridge.capabilities();
  require_contains(caps, "\"im_send\"");
  require_contains(caps, "\"im_query_dialog_list\"");
  require_contains(caps, "\"im_query_messages_by_channel\"");
  if (caps.find("im:post:send-failed") != std::string::npos) {
    std::cerr << "capabilities must not expose im:post:send-failed\n";
    return 1;
  }

  int32_t rc = bridge.call(
      "im_send",
      "{\"channelId\":\"01hq0000000000000000000000\","
      "\"temporaryId\":\"tmp-cpp-smoke-1\","
      "\"text\":\"hello from cpp smoke\","
      "\"type\":\"TEXT\"}");
  if (rc != FFI_OK) {
    std::cerr << "im_send returned rc=" << rc << "\n";
    return 1;
  }

  auto batches = bridge.drainEvents();
  bool saw_sending = false;
  for (const auto& batch : batches) {
    if (batch.find("im:post:sending") != std::string::npos &&
        batch.find("tmp-cpp-smoke-1") != std::string::npos) {
      saw_sending = true;
    }
  }
  if (!saw_sending) {
    std::cerr << "did not observe im:post:sending for tmp-cpp-smoke-1\n";
    return 1;
  }

  bridge.dispose();
  std::cout << "cpp core bridge smoke passed\n";
  return 0;
}
```

- [ ] **Step 2: Add run script**

Create `scripts/run-cpp-smoke.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/build"

c++ -std=c++17 \
  -I"$ROOT/cpp" \
  -I"$ROOT/third_party/helix/include" \
  "$ROOT/smoke/cpp_core_bridge_smoke.cpp" \
  "$ROOT/cpp/CoreBridge.cpp" \
  -L"$ROOT/third_party/helix/lib" \
  -lhelix_driver_ffi \
  -o "$ROOT/build/cpp_core_bridge_smoke"

export DYLD_LIBRARY_PATH="$ROOT/third_party/helix/lib:${DYLD_LIBRARY_PATH:-}"
export LD_LIBRARY_PATH="$ROOT/third_party/helix/lib:${LD_LIBRARY_PATH:-}"
"$ROOT/build/cpp_core_bridge_smoke"
```

- [ ] **Step 3: Run S0 smoke**

```bash
cd /workspace/c/mobile-qucik-c++
chmod +x scripts/run-cpp-smoke.sh
make cpp-smoke
```

Expected:

```text
cpp core bridge smoke passed
```

- [ ] **Step 4: Commit C++ smoke**

```bash
cd /workspace/c/mobile-qucik-c++
git add smoke/cpp_core_bridge_smoke.cpp scripts/run-cpp-smoke.sh Makefile
git commit -m "test: 增加 CoreBridge 真实 FFI 冒烟"
```

---

### Task 4: QuickJS native class 注册

**Files:**
- Create: `/workspace/c/mobile-qucik-c++/scripts/build-quickjs.sh`
- Create: `/workspace/c/mobile-qucik-c++/quickjs/bind_core_bridge.cpp`
- Create: `/workspace/c/mobile-qucik-c++/quickjs/quickjs_host.cpp`
- Create: `/workspace/c/mobile-qucik-c++/scripts/run-quickjs-smoke.sh`
- Create: `/workspace/c/mobile-qucik-c++/smoke/quickjs_core_bridge_smoke.js`

**Interfaces:**
- Consumes: `mobile::CoreBridge`
- Produces:
  - JS global `CoreBridge`
  - methods `version()`, `capabilities()`, `call(name,payload)`, `query(name,payload)`, `drainEvents()`, `dispose()`

- [ ] **Step 1: Add QuickJS artifact script**

Create `scripts/build-quickjs.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
QUICKJS_ROOT="${QUICKJS_ROOT:-/workspace/c/quickjs}"
OUT="$ROOT/third_party/quickjs"

if [ ! -d "$QUICKJS_ROOT" ]; then
  echo "QuickJS source not found: $QUICKJS_ROOT" >&2
  exit 1
fi

make -C "$QUICKJS_ROOT" libquickjs.a

mkdir -p "$OUT/include" "$OUT/lib"
cp "$QUICKJS_ROOT/quickjs.h" "$OUT/include/quickjs.h"
cp "$QUICKJS_ROOT/quickjs-libc.h" "$OUT/include/quickjs-libc.h"
cp "$QUICKJS_ROOT/libquickjs.a" "$OUT/lib/libquickjs.a"

echo "quickjs artifacts copied to $OUT"
```

- [ ] **Step 2: Implement binding with string JSON boundary**

Create `quickjs/bind_core_bridge.cpp`. The binding converts JS objects to JSON strings via `JSON.stringify` before calling C++ and parses returned JSON with `JSON.parse`.

Required exported symbol:

```cpp
extern "C" void register_core_bridge(JSContext* ctx);
```

Required behavior:

- `new CoreBridge({ apiBaseUrl, wsUrl })` creates C++ bridge.
- `core.version()` returns parsed JSON.
- `core.capabilities()` returns parsed JSON.
- `core.call(name, payload)` returns `{ ok: true, rc }` when rc is `FFI_OK`.
- `core.query(name, payload)` returns parsed JSON.
- `core.drainEvents()` returns an array of raw batch strings.
- `core.dispose()` destroys the handle.

Error mapping:

```js
{ code: "CORE_BRIDGE_ERROR", message: "..." }
```

- [ ] **Step 3: Implement command-line QuickJS host**

Create `quickjs/quickjs_host.cpp`:

```cpp
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

extern "C" {
#include "quickjs.h"
}

extern "C" void register_core_bridge(JSContext* ctx);

int main(int argc, char** argv) {
  if (argc != 2) {
    std::cerr << "usage: quickjs_host <script.js>\n";
    return 2;
  }

  std::ifstream file(argv[1]);
  if (!file) {
    std::cerr << "cannot open script: " << argv[1] << "\n";
    return 2;
  }
  std::stringstream buffer;
  buffer << file.rdbuf();
  std::string code = buffer.str();

  JSRuntime* rt = JS_NewRuntime();
  JSContext* ctx = JS_NewContext(rt);
  register_core_bridge(ctx);

  JSValue result = JS_Eval(ctx, code.data(), code.size(), argv[1], JS_EVAL_TYPE_MODULE);
  if (JS_IsException(result)) {
    JSValue exception = JS_GetException(ctx);
    const char* msg = JS_ToCString(ctx, exception);
    std::cerr << "JS exception: " << (msg ? msg : "<non-string>") << "\n";
    JS_FreeCString(ctx, msg);
    JS_FreeValue(ctx, exception);
    JS_FreeValue(ctx, result);
    JS_FreeContext(ctx);
    JS_FreeRuntime(rt);
    return 1;
  }

  JS_FreeValue(ctx, result);
  JS_FreeContext(ctx);
  JS_FreeRuntime(rt);
  return 0;
}
```

- [ ] **Step 4: Add CoreBridge JS smoke**

Create `smoke/quickjs_core_bridge_smoke.js`:

```js
const core = new CoreBridge({
  apiBaseUrl: "http://127.0.0.1:8066",
  wsUrl: "ws://127.0.0.1:8066/api/cses/websocket"
});

const version = core.version();
if (version.abi !== 1) {
  throw new Error("expected abi=1");
}

const caps = core.capabilities();
if (!caps.commands.includes("im_send")) {
  throw new Error("missing im_send");
}
if (caps.events.includes("im:post:send-failed")) {
  throw new Error("send-failed must not be public P0 event");
}

const result = core.call("im_send", {
  channelId: "01hq0000000000000000000000",
  temporaryId: "tmp-qjs-core-1",
  text: "hello from quickjs core smoke",
  type: "TEXT"
});
if (!result.ok) {
  throw new Error("im_send was not accepted");
}

const batches = core.drainEvents();
if (!batches.some((batch) => batch.includes("im:post:sending") && batch.includes("tmp-qjs-core-1"))) {
  throw new Error("missing im:post:sending event");
}

core.dispose();
```

- [ ] **Step 5: Add QuickJS smoke script**

Create `scripts/run-quickjs-smoke.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$ROOT/build"

c++ -std=c++17 \
  -I"$ROOT/cpp" \
  -I"$ROOT/third_party/helix/include" \
  -I"$ROOT/third_party/quickjs/include" \
  "$ROOT/quickjs/quickjs_host.cpp" \
  "$ROOT/quickjs/bind_core_bridge.cpp" \
  "$ROOT/cpp/CoreBridge.cpp" \
  "$ROOT/third_party/quickjs/lib/libquickjs.a" \
  -L"$ROOT/third_party/helix/lib" \
  -lhelix_driver_ffi \
  -lm -ldl -lpthread \
  -o "$ROOT/build/quickjs_host"

export DYLD_LIBRARY_PATH="$ROOT/third_party/helix/lib:${DYLD_LIBRARY_PATH:-}"
export LD_LIBRARY_PATH="$ROOT/third_party/helix/lib:${LD_LIBRARY_PATH:-}"
"$ROOT/build/quickjs_host" "$ROOT/smoke/quickjs_core_bridge_smoke.js"
```

- [ ] **Step 6: Run S1 smoke**

```bash
cd /workspace/c/mobile-qucik-c++
chmod +x scripts/build-quickjs.sh scripts/run-quickjs-smoke.sh
make quickjs-smoke
```

Expected: command exits `0`.

- [ ] **Step 7: Commit QuickJS binding**

```bash
cd /workspace/c/mobile-qucik-c++
git add scripts/build-quickjs.sh scripts/run-quickjs-smoke.sh quickjs/bind_core_bridge.cpp quickjs/quickjs_host.cpp smoke/quickjs_core_bridge_smoke.js Makefile
git commit -m "feat: 注册 QuickJS CoreBridge"
```

---

### Task 5: JS `im.send(...)` facade 和业务 smoke

**Files:**
- Create: `/workspace/c/mobile-qucik-c++/js/core_bridge.js`
- Create: `/workspace/c/mobile-qucik-c++/js/im_api.js`
- Create: `/workspace/c/mobile-qucik-c++/smoke/quickjs_im_send_smoke.js`
- Modify: `/workspace/c/mobile-qucik-c++/scripts/run-quickjs-smoke.sh`

**Interfaces:**
- Consumes: JS global `CoreBridge`
- Produces:
  - `createCoreBridge(config)`
  - `createImApi(core)`
  - `im.send({ channelId, text, type, temporaryId? })`

- [ ] **Step 1: Add JS low-level helper**

Create `js/core_bridge.js`:

```js
export function createCoreBridge(config = {}) {
  const core = new CoreBridge(config);
  const listeners = new Set();

  function pumpEvents() {
    const batches = core.drainEvents();
    for (const batch of batches) {
      for (const listener of listeners) {
        listener(batch);
      }
    }
    return batches;
  }

  return {
    raw: core,
    version: () => core.version(),
    capabilities: () => core.capabilities(),
    call: (name, payload = {}) => core.call(name, payload),
    query: (name, payload = {}) => core.query(name, payload),
    drainEvents: pumpEvents,
    onEvents(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    dispose() {
      listeners.clear();
      core.dispose();
    }
  };
}
```

- [ ] **Step 2: Add IM facade**

Create `js/im_api.js`:

```js
function makeTemporaryId() {
  return `tmp-mobile-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export function createImApi(core) {
  const caps = core.capabilities();
  if (caps.abi !== 1) {
    throw new Error("helix abi mismatch");
  }
  if (!caps.commands.includes("im_send")) {
    throw new Error("helix capability missing im_send");
  }
  if (caps.events.includes("im:post:send-failed")) {
    throw new Error("send-failed must not be public P0 event");
  }

  return {
    async send(input) {
      if (!input || !input.channelId || !input.text) {
        throw new Error("send requires channelId and text");
      }
      const temporaryId = input.temporaryId || makeTemporaryId();
      const payload = {
        channelId: input.channelId,
        temporaryId,
        text: input.text,
        type: input.type || "TEXT"
      };

      const accepted = core.call("im_send", payload);
      if (!accepted || accepted.ok !== true) {
        throw new Error("im_send was not accepted");
      }

      const batches = core.drainEvents();
      const sawSending = batches.some((batch) =>
        batch.includes("im:post:sending") && batch.includes(temporaryId)
      );
      if (!sawSending) {
        throw new Error(`missing optimistic sending event for ${temporaryId}`);
      }

      return { temporaryId, status: "sending" };
    },

    onEvents(listener) {
      return core.onEvents(listener);
    }
  };
}
```

- [ ] **Step 3: Add facade smoke**

Create `smoke/quickjs_im_send_smoke.js`:

```js
import { createCoreBridge } from "../js/core_bridge.js";
import { createImApi } from "../js/im_api.js";

const core = createCoreBridge({
  apiBaseUrl: "http://127.0.0.1:8066",
  wsUrl: "ws://127.0.0.1:8066/api/cses/websocket"
});

const im = createImApi(core);
const receipt = await im.send({
  channelId: "01hq0000000000000000000000",
  temporaryId: "tmp-qjs-im-send-1",
  text: "hello from im.send smoke",
  type: "TEXT"
});

if (receipt.temporaryId !== "tmp-qjs-im-send-1") {
  throw new Error("temporaryId mismatch");
}
if (receipt.status !== "sending") {
  throw new Error("receipt must be optimistic sending");
}

core.dispose();
```

- [ ] **Step 4: Run both QuickJS smokes**

Modify `scripts/run-quickjs-smoke.sh` so it runs both files:

```bash
"$ROOT/build/quickjs_host" "$ROOT/smoke/quickjs_core_bridge_smoke.js"
"$ROOT/build/quickjs_host" "$ROOT/smoke/quickjs_im_send_smoke.js"
```

Run:

```bash
cd /workspace/c/mobile-qucik-c++
make quickjs-smoke
```

Expected: command exits `0`.

- [ ] **Step 5: Commit JS facade**

```bash
cd /workspace/c/mobile-qucik-c++
git add js/core_bridge.js js/im_api.js smoke/quickjs_im_send_smoke.js scripts/run-quickjs-smoke.sh
git commit -m "feat: 暴露 im.send QuickJS facade"
```

---

### Task 6: 多端联调运行手册和环境验收

**Files:**
- Create: `/workspace/c/mobile-qucik-c++/docs/多端联调运行手册.md`
- Modify: `/workspace/c/mobile-qucik-c++/README.md`
- Modify: `/workspace/c/mobile-qucik-c++/Makefile`

**Interfaces:**
- Consumes: S0/S1/S2 smoke
- Produces: 可复跑 runbook，明确 S3/S4 移动端环境缺口。

- [ ] **Step 1: Create runbook**

Create `docs/多端联调运行手册.md`:

```markdown
# 多端联调运行手册

## 目录职责

- helix: `/System/Volumes/Data/workspace/rust/helix`
- mobile C++/QuickJS: `/workspace/c/mobile-qucik-c++`
- QuickJS source: `/workspace/c/quickjs`
- Go IM server: `/System/Volumes/Data/workspace/golang/cses-im-server`

## S0: C++ 直连真实 helix-driver-ffi

```bash
cd /workspace/c/mobile-qucik-c++
make cpp-smoke
```

通过标准：

- `helix_version` 返回 `abi=1`
- `helix_capabilities` 包含 `im_send`
- `helix_capabilities` 不包含 `im:post:send-failed`
- `im_send` 返回 `FFI_OK`
- 事件批出现 `im:post:sending`

## S1: QuickJS CoreBridge smoke

```bash
cd /workspace/c/mobile-qucik-c++
make quickjs-smoke
```

通过标准：

- JS 能 `new CoreBridge()`
- JS 能调用 `version/capabilities/call/query/drainEvents/dispose`
- JS 能观察事件批

## S2: JS facade smoke

```bash
cd /workspace/c/mobile-qucik-c++
make quickjs-smoke
```

通过标准：

- JS 调用 `im.send(...)`
- Flutter 调用方不需要知道 `im_send`
- 返回 `{ temporaryId, status: "sending" }`
- 不出现 `im:post:send-failed`

## S3: Android / iOS 原生产物验收

当前本机若没有 Android NDK 或 Xcode，记录为环境缺口，不阻塞 S0-S2。

通过标准：

- Android 加载 QuickJS + C++ bridge + helix native library
- iOS 加载 QuickJS + C++ bridge + helix native library
- 架构切片和 loader path 正确

## S4: Flutter 端到端验收

当前本机若没有 Flutter e2e 环境，记录为环境缺口，不阻塞 S0-S2。

通过标准：

- Flutter 点击发送
- JS 调用 `im.send(...)`
- QuickJS 调 C++ `CoreBridge.call`
- C++ 调 Rust `helix_command`
- Rust 产生 `im:post:sending`
- Go WS echo 后 Rust 产生 `im:post:received`
- Flutter UI 完成乐观行对账
```

- [ ] **Step 2: Add verify command**

Ensure `Makefile` includes:

```make
verify: cpp-smoke quickjs-smoke
```

Run:

```bash
cd /workspace/c/mobile-qucik-c++
make verify
```

Expected: S0-S2 pass on command line. If Android/iOS/Flutter tools are missing, document exact missing command in `docs/多端联调运行手册.md`.

- [ ] **Step 3: Verify helix boundary remains clean**

Run in helix:

```bash
cd /System/Volumes/Data/workspace/rust/helix
find crates/helix-driver-ffi -path '*mobile_bridge*' -o -path '*mobile_core_bridge*'
git status --porcelain=v1 --untracked-files=all
```

Expected:

- `find` has no output.
- helix only contains docs/contract changes, no external implementation files.

- [ ] **Step 4: Commit runbook**

```bash
cd /workspace/c/mobile-qucik-c++
git add README.md Makefile docs/多端联调运行手册.md
git commit -m "docs: 补充多端联调运行手册"
```

---

## Self-Review

Spec coverage:

- C++ `CoreBridge`：Task 2。
- QuickJS native class：Task 4。
- `CoreBridge.call/query/capabilities/onEvents/dispose` JS 公开面：Task 4，其中事件用 `drainEvents()` 覆盖同一队列语义。
- JS `im.send(...)` 业务 facade：Task 5。
- Android / iOS 构建脚本与原生产物打包：Task 6 先写验收手册，环境就绪后再拆平台计划。
- C++ 直连真实 `helix-driver-ffi` smoke：Task 3。
- QuickJS 每个公开封装方法 smoke：Task 4。
- JS 与事件批驱动的 Flutter 发消息端到端验证：Task 6 记录 S4 验收标准，当前环境缺口不阻塞 S0-S2。
- 不在 helix 中实现外部文件：Global Constraints + Task 6 boundary check。

Placeholder scan:

- 本计划没有使用 TBD 或空泛占位。
- Android/iOS/Flutter 环境缺口以 S3/S4 验收层记录，不伪装成本机已完成。

Type consistency:

- C++ 层统一使用 `std::string` JSON 边界。
- QuickJS binding 负责 JS object <-> JSON string。
- JS facade 只调用 `core.call("im_send", payload)` 和 `core.drainEvents()`。
