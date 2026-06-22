# Helix Flutter FFI 桌面 Demo — 设计 spec（ROADMAP F4.5）

> 状态：**已批准**（用户拍板方案 B + 三方专家 review 收敛）。日期：2026-06-18。分支：dev-zlc。
> 关联：ROADMAP F4.5「真 Flutter app `dart:ffi`→C++→Rust 冒烟」+ M3 追加「FFI Flutter 冒烟 + abort 语义验证」。
> 评审：cpp-rust-systems-dev（🟡有条件可行）· rust-runtime-agnostic-core-dev（🟢符合约束·确认零 Rust 改动）· rust-dev（🟡有条件正确·无 UAF/竞态）。

---

## 1. 目标 / 非目标

**目标**：macOS 桌面端跑通完整链「形状」——
`Flutter UI → 最小 C++ shim → helix cdylib →（pump 线程 step → Effect::Emit → on_domain_events_batch 批回调）→ shim → Dart listener → Flutter UI`，
由真实按钮点击驱动，**离线**（本地 optimistic `im:post:sending`，不接服务器）。外加可跑的 `integration_test` 冒烟 + Maestro 空骨架。

**非目标（YAGNI）**：不接真服务器/sync；不做 `helix_query`（仍 stub）；不做产品级 UI；不搞打包签名/公证；不做 Android/iOS/Linux/Windows；**helix Rust crates 零改动**（headless 守卫兑现）。

---

## 1.5 环境约束（2026-06-18 实测）+ C 路线分阶段交付

`flutter doctor` 实测：`cmake`/`clang++`/Flutter 3.44.1 ✅，但**完整 Xcode 未装**（只有 Command Line Tools）+ **CocoaPods 未装** → `flutter run -d macos` / `flutter test integration_test -d macos` **当前无法运行**（macOS Flutter 桌面强依赖完整 Xcode）。用户拍板走 **C 路线：分两阶段**：

- **阶段 1（现在做，不依赖 Xcode）**：C++ shim + Dart FFI 绑定 + `build_native.sh`（`cargo build --release` + `clang`，不碰 Xcode）+ **`dart run` 纯 console dart:ffi→shim→cdylib→listener 异步回调**证完整链路 + `lib/main.dart` UI / `integration_test` / maestro 骨架**代码就绪** + helix 仓文档。
- **阶段 2（待用户装完整 Xcode + CocoaPods 后一键终验）**：`flutter test integration_test -d macos` 跑绿。

故下文 §11 P3 拆为 **P3a（console 链路验证，阶段 1 跑绿）** + **P3b（macos integration_test，阶段 2 BLOCKED-on-Xcode）**；§13 验收标准中 `-d macos` 项标注待 Xcode。**console 冒烟 = 阶段 1 的链路真证**（dart:ffi→C++ shim→cdylib→pump→Emit→batch→listener→dart 全链 + 异步跨线程，仅缺 Flutter widget 渲染层）。

---

## 2. 背景与现状（为什么这是纯消费、零 Rust 改动）

helix-driver-ffi 已导出完整 C ABI（`cdylib + staticlib + rlib`），cpp→rust→cpp 冒烟（`smoke/smoke.cpp` + CMake/CTest）已通过。demo 全链路正是 e2e 测试 `command_emits_domain_event_through_batch_callback`（`api.rs:651`）已验证的路径：

- `helix_create_with_network` 起 pump 线程 → `helix_command(im_send_message)` → pump step → `Effect::Emit(im:post:sending)` → `FfiEventSink` 攒批 → `on_domain_events_batch` → `helix_destroy`（join pump）。
- `now_ms` 固定值 = `clock.rs` 既有 `FfiClock`（vtable 字段已就位）。
- 批 wire = `batch.rs` 既有格式，无需新导出。
- offline optimistic emit 不依赖网络（连不上 WS 仍进泵，`pump.rs:110`）→ 离线可跑。
- `helix_query` 仍 honest stub（`FFI_UNIMPLEMENTED`），demo 是 emit 冒烟、不读状态 → **不触达**。

**复用的 FFI 线程事实**（来自仓外无关 POC `ffi_stack_poc` 的实证，仅借经验不依赖其代码）：
`NativeCallable.listener` = 异步投递，**从外来（pump）线程调用安全**；`NativeCallable.isolateLocal` = 同步，从外来线程调用**必崩**（`Cannot invoke native callback outside an isolate`）。→ 这正是 C++ shim 不可省的根因。

---

## 3. 方案 B 架构（NativeCallable.listener 推送）+ 关键不变量

```
Dart: NativeCallable.listener(handler) → nativeFunction (C 函数指针)
  └─ helix_shim_create(api, ws, dartCb)
       └─ shim 造 CppHostVtable{ on_domain_events_batch=shim_on_batch, now_ms=固定值, ctx=shim }
            └─ helix_create_with_network(...) → HelixHandle*

点击发送: Dart helix_shim_command(name,payload) → shim → helix_command(...)（非阻塞 tick_tx.send）

事件回流（pump 线程）:
  cdylib → vtable.on_domain_events_batch(ctx, ptr, len)  == shim_on_batch
    └─ shim malloc(len) + memcpy(ptr..ptr+len)  到稳定堆      ← ★关键不变量①
    └─ dartCb(copyPtr, len)   (listener: 异步投递地址整数, 立即返回, noexcept)  ← ★②
Dart 事件循环: handler(copyPtr, len)
    └─ Uint8List.fromList(p.asTypedList(len)) 拷出
    └─ helix_shim_free_batch(copyPtr)        ← 与 shim malloc 配对, 无状态 free  ← ★③
    └─ decode_batch（循环 u32 count + [u32 len, bytes] LE）→ setState → ListView 出现 im:post:sending
```

**★ 关键不变量**：
- **①稳定化**：helix 契约规定 vtable 回调里 `ptr` 仅回调期间有效；`.listener` 异步，Dart handler 执行时 pump buffer 已失效 → **shim 必须在 `shim_on_batch` 里 `malloc`+`memcpy` 到稳定堆**，把稳定指针交给 Dart。
- **②整批一次 + noexcept**（HX-C007 + HX-C006）：shim **整批一块**一次性传 Dart（**禁** per-event 多次调 Dart listener，否则把批打散成逐条跨边界 = 违 HX-C007）；`shim_on_batch` 回调体 `noexcept` + try/catch 兜底，C++ 异常绝不外逸跨 `extern "C"`。
- **③free_batch 无状态**（rust-dev 铁律）：`helix_shim_free_batch` 只 `free(ptr)`，**绝不解引用 shim/handle** → 故「join 后 handle 已 destroy、Dart 仍调 free_batch」**不是 UAF**；内存独立于 handle。全程 C `malloc/free`，不混 C++ `new`。

---

## 4. shim 扁平 ABI（Dart FFI 绑定面）

```c
// helix_shim.h — len 统一 uintptr_t 与 helix vtable 一致（cpp-rust review 修正）
typedef void (*HelixDartBatchCb)(const uint8_t* ptr, uintptr_t len);

void*    helix_shim_create(const char* api, const char* ws, HelixDartBatchCb cb);
int32_t  helix_shim_command(void* shim, const char* name, const char* payload);
void     helix_shim_free_batch(const uint8_t* ptr);  // 仅释放 shim_on_batch malloc 的批, 非 vtable ptr
void     helix_shim_destroy(void* shim);
```

Dart 侧签名对齐：`NativeCallable<Void Function(Pointer<Uint8>, UintPtr)>`（`UintPtr`，非 `IntPtr`）。

---

## 5. 批 wire 格式（`batch.rs` 既有，Dart 直接按此解码）

```text
u32 count (LE)
每条: u32 len (LE) + len 字节 DomainEventBytes 原文（helix-im 序列化的 {"event":...,"data":...} JSON）
```
冒烟断言：批字节里子串含 `im:post:sending` 即通过（等价 smoke.cpp 的 `b.find`）。UI 展示按 count/len 拆条目。

---

## 6. 三方 review 收敛的修正清单（已并入本设计）

| # | 修正 | 来源 | 落点 |
|---|---|---|---|
| R1 | **demo/test 一律链 release dylib**（release 已 `panic="abort"`）→ helix 仓零改 Cargo.toml；叠加 shim 回调 `noexcept`+try/catch | 三方 | `build_native.sh` 用 `cargo build --release`；shim |
| R2 | shim `len` 用 `uintptr_t`、Dart 用 `UintPtr` | cpp-rust | §4 |
| R3 | 整批一次投 Dart，Dart 侧才 `decode_batch` 拆条目（禁逐条跨边界） | runtime-agnostic | §3②、shim、helix_ffi.dart |
| R4 | `free_batch` 无状态、内存独立于 handle；全程 C malloc/free | rust-dev | §3③、shim |
| R5 | 销毁 6 步（见 §8）；`_cb.close()` 前 drain Dart port queue 避免末批 leak；冒烟显式接受丢末批 | rust-dev | main.dart 销毁路径 |
| R6 | 链接 cdylib + `@rpath`（install_name + `INSTALL_RPATH=@loader_path/../Frameworks` + `install_name_tool -change`） | cpp-rust | build_native.sh |
| R7 | `FFI_BUSY` 生产退避重试（禁静默丢）/ `FFI_NULL` 已销毁静默；demo `<0` 一律 log | rust-dev | helix_ffi.dart 注释 |
| R8 | 仓内只加 pointer.md + ffi `CLAUDE.md`/`AGENTS.md` 各 1 行（镜像成对、diff 空，`structure-gate.sh` 卡） | runtime-agnostic | §10 |
| R9 | 写 SESSION §0 EXPECT 可证伪行（§4.8 达实质门槛） | runtime-agnostic | §10 |

---

## 7. 目录结构（`/workspace/flutterWorkspace/helix_ffi_demo/`，全新，不碰 `ffi_stack_poc`）

```
helix_ffi_demo/
  pubspec.yaml                  # ffi + integration_test/flutter_test dev_dep
  lib/
    main.dart                   # 最小 UI：发送按钮 + 事件 ListView + 销毁路径
    helix_ffi.dart              # FFI 绑定：DynamicLibrary.open + NativeCallable.listener + decode_batch
  native/
    helix_shim.h                # 扁平 ABI（§4）
    helix_shim.cpp              # 最小 C++ host：构 vtable / 转发 / 拷批 / free / noexcept 兜底
    helix.hpp                   # 从 crates/helix-driver-ffi/helix.hpp 拷入（cbindgen 产物）
  macos/                        # flutter create --platforms=macos 产物
  integration_test/
    chain_smoke_test.dart       # tap 发送 → pump 重试等异步 listener → 断言 im:post:sending 上屏
  maestro/
    smoke_flow.yaml             # 空骨架（launchApp / tapOn 发送 / assertVisible im:post:sending）
    README.md                   # brew install maestro 指引 + 标「本机未装、未验证」
  tool/
    build_native.sh             # cargo build --release ffi + clang 编 shim 链 cdylib + install_name_tool + 拷 bundle
  README.md                     # 复跑步骤 + 链路图 + B 模型 ptr 生命周期/销毁顺序说明
```

---

## 8. 链接与 bundle 策略（macOS）

- `cargo build --release -p helix-driver-ffi` → `target/release/libhelix_driver_ffi.dylib`（panic=abort）。
- `clang++ -std=c++17 -O2` 编 `helix_shim.cpp` → `libhelix_shim.dylib`，链接 `libhelix_driver_ffi.dylib`（沿用 `smoke/CMakeLists.txt`：`-L target/release -lhelix_driver_ffi`）。**保留 C++ 异常**（不加 `-fno-exceptions`），`shim_on_batch` 内 try/catch 把任何异常吞在回调边界内，绝不外逸跨 `extern "C"`。
- install_name：`libhelix_shim.dylib` → `@rpath/libhelix_shim.dylib`；`install_name_tool -change` 把对 helix dylib 的引用改 `@rpath/libhelix_driver_ffi.dylib`。
- 两个 dylib 拷进 `.app/Contents/Frameworks/`；Runner 的 `@rpath` 含 `@executable_path/../Frameworks`。Dart `DynamicLibrary.open('libhelix_shim.dylib')` 经 bundle rpath 解析。
- **开发期回退**：若 bundle 集成卡壳，先按绝对路径 `DynamicLibrary.open('<repo>/target/release/...')`（dev only），integration_test 通过 `DYLD_LIBRARY_PATH=target/release` 兜底（对齐 smoke CMake 的 ctest 环境）。

**销毁 6 步**（rust-dev 锁定，唯一同步屏障 = join）：
```
① shim 停发命令 → ② helix_shim_destroy→helix_destroy: drop tick_tx → send shutdown → join pump_thread
   （join 只锚「Rust→C++ 同步回调已返回」，不锚「Dart 异步队列已空」）
→ ③ Rust 不再触达 ctx → ④ Dart drain port queue（await 一拍）处理残留批（含 free_batch）
→ ⑤ _cb.close() → ⑥ shim 释放 ctx/自身
```

---

## 9. 测试

- **integration_test（可跑核心）**：`flutter test integration_test -d macos`（app debug 无妨，链的是 release helix dylib + shim）→ 启动真 app → `tester.tap(发送按钮)` → `tester.pump` 重试循环等异步 listener → 断言事件 ListView 含 `im:post:sending`。等价把 smoke.cpp 的 200×10ms 轮询换成 widget tester。
- **Maestro 空骨架**：`smoke_flow.yaml`（`launchApp` / `tapOn: 发送` / `assertVisible: im:post:sending`）+ README（`brew install maestro` 指引，标本机未装/未验证）。不要求本机跑。

---

## 10. helix 仓内唯一改动（纯文档，零 Rust 改动）

1. 新增 `crates/helix-driver-ffi/docs/flutter-demo-pointer.md`：头部明示「Flutter 工程在仓外 `/workspace/flutterWorkspace/helix_ffi_demo`，**本仓不含 Dart/C++ 代码**」+ 链路图 + B 模型 ptr 生命周期/销毁顺序摘要 + 指向本 spec。
2. ffi crate `CLAUDE.md` + `AGENTS.md` 各加一行索引（**镜像铁律：成对、内容一致、`diff` 为空**，`structure-gate.sh` 会卡）。
3. SESSION §0 EXPECT 行（§4.8，开局写，可证伪）：
   ```
   🎯 EXPECT 只动 docs/flutter-demo-pointer.md + ffi CLAUDE.md/AGENTS.md 各 1 行 | cargo check -p helix-driver-ffi 仍过 | helix-core/helix-im 零改 | structure-gate 镜像闸绿 | 赌注 低 | 可逆 是 | @2026-06-18
   ```

---

## 11. 实现阶段拆解（= 实现计划，映射 Workflow phases）

| Phase | 内容 | 依赖 | 验证 |
|---|---|---|---|
| **P1 脚手架（并行）** | A1 `flutter create --platforms=macos helix_ffi_demo` + pubspec dev_dep；A2 `cargo build --release -p helix-driver-ffi` 产 dylib + 固化「ABI 契约卡」（6 导出签名 + batch wire + vtable）；A3 helix 仓三文档（pointer + 镜像 + SESSION EXPECT）；A4 maestro 骨架 | 无 | A1 `flutter doctor`/工程生成；A2 dylib 存在 + `nm` 见符号；A3 `structure-gate.sh` 镜像绿；A4 yaml 合法 |
| **P2 核心实现（连贯一 agent）** | `native/helix_shim.{h,cpp}`（noexcept/uintptr_t/整批 malloc/无状态 free）+ `lib/helix_ffi.dart`（listener/UintPtr/decode_batch/销毁）+ `lib/main.dart`（按钮+ListView+销毁 6 步）+ `tool/build_native.sh`（release+clang+install_name_tool+拷 bundle） | A1 骨架 + A2 契约卡 | `build_native.sh` 编译链接通过；`nm`/`otool -L` 验 rpath |
| **P3a 编译链接 + console 链路验证（阶段 1）** | 跑 `build_native.sh` → 写 `bin/console_smoke.dart`（`dart run` 纯 dart:ffi 驱动：create→command→listener 异步收批→断言含 `im:post:sending`→destroy）→ `dart run bin/console_smoke.dart` 跑绿 | P2 | console_smoke PASS + `otool -L` 验 rpath，不碰 Xcode |
| **P3b macos integration_test（阶段 2，BLOCKED-on-Xcode）** | 写 `integration_test/chain_smoke_test.dart`（tap→pump→断言上屏）+ `lib/main.dart` UI 代码就绪；`flutter test integration_test -d macos` 待用户装完整 Xcode+CocoaPods 后跑 | P2 + 用户装 Xcode | 代码就绪即算阶段 1 完成；终验待 Xcode |
| **P4 终验 + 收口** | helix 仓零回归：`cargo check -p helix-driver-ffi` + `bash scripts/cpp-smoke.sh`（确认未破现有）+ `structure-gate.sh`；SESSION §0 写 `✅ DONE`；EXPECT 对账（命中/`⚡ DRIFT-PRED`） | P1–P3 | 全绿 + 终态行落 SESSION |

---

## 12. 风险与回滚

| 风险 | 缓解 |
|---|---|
| macOS Flutter native dylib 集成（rpath/bundle/xcode run script）首次跑不通 | P3 agent 在自身 context 迭代；回退绝对路径 + `DYLD_LIBRARY_PATH`（dev）先证链路，再补 bundle |
| release 编译 helix 依赖树（tokio/rusqlite/helix-im）首次慢（数分钟） | P1-A2 提前并行起；可接受一次性成本 |
| debug build panic=unwind UB | R1：一律链 release dylib + shim noexcept 兜底 |
| 末批 leak / `_cb.close()` 丢消息 | R5：drain 一拍；冒烟显式接受 |
| 误改 helix 仓根 SESSION/CLAUDE（§6.5 黑名单） | 仓内仅 ffi crate 子目录 + docs + SESSION §0（主对话写，不让 worktree agent 碰根文件） |

**回滚**：Flutter 工程在仓外，删目录即回滚；helix 仓改动是 3 行文档，`git revert` 单 commit 即净。

---

## 13. 验收标准（Definition of Done）

- [ ] **（阶段 1）** `dart run bin/console_smoke.dart` 绿，dart:ffi→shim→cdylib→listener 异步收批断言含 `im:post:sending`。
- [ ] **（阶段 2 · BLOCKED-on-Xcode）** `flutter test integration_test -d macos` 绿，断言事件 ListView 含 `im:post:sending`（代码就绪，待用户装完整 Xcode+CocoaPods）。
- [ ] `tool/build_native.sh` 一键 release 构建 + 链接成功（`cargo build --release` + `clang`，不碰 Xcode）。
- [ ] `cargo check -p helix-driver-ffi` 仍过 + `scripts/cpp-smoke.sh` 仍绿（helix 零回归）。
- [ ] `structure-gate.sh` 镜像闸绿（ffi CLAUDE/AGENTS 成对）。
- [ ] helix-core/helix-im **零改**（`git diff` 仅 docs + ffi CLAUDE/AGENTS + SESSION §0）。
- [ ] Maestro 骨架 + README 落地（不要求本机跑）。
- [ ] SESSION §0 落 `✅ DONE` 终态行 + EXPECT 对账。
