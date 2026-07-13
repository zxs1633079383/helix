# Flutter FFI 桌面 Demo — 仓外指针（F4.5 · C 路线）

> ⚠️ **Flutter 工程在仓外** `/workspace/flutterWorkspace/helix_ffi_demo`。
> **本仓（helix）不含任何 Dart / C++ demo 代码** —— helix 保持 headless（沿用 GOAL §1 sans-IO + AGENTS.md §0 三层架构守卫）。
> 本文件只是**指针**：把仓外 demo 工程指回本 crate 已导出的 C ABI，并摘录跨边界的内存/销毁不变量。
> **当前真源 →** [`SPEC-full-takeover.md`](./SPEC-full-takeover.md)（FFI 接管边界）+ [`移动端Command-Query-Event契约SPEC.md`](./移动端Command-Query-Event契约SPEC.md)（对外契约）。已删除的 2026-06-18 Superpowers 阶段稿只保留在 Git 历史。

---

## 这是什么 / 为什么零 Rust 改动

macOS 桌面端跑通完整链「形状」——真实按钮点击驱动、**离线**（本地 optimistic `im:post:sending`，不接服务器）：

```
Flutter UI → 最小 C++ shim → helix cdylib →（pump 线程 step → Effect::Emit → on_domain_events_batch 批回调）→ shim → Dart listener → Flutter UI
```

全链路正是 e2e 测试 `command_emits_domain_event_through_batch_callback`（`tests/.../api` 已验证路径）的真实消费：
`helix_create_with_network` 起 pump 线程 → `helix_command(im_send)`（FFI alias 到内部 send handler）→ pump step → `Effect::Emit(im:post:sending)` → `FfiEventSink` 攒批 → `on_domain_events_batch`（HX-C007 整批一次）→ `helix_destroy`（join pump）。
demo 是 **emit 冒烟、不读状态**，`helix_query` 仍 honest stub（`FFI_UNIMPLEMENTED`）→ 不触达 → **本 crate 零改动**（headless 守卫兑现）。

---

## 方案 B 链路图（NativeCallable.listener 推送）

```
Dart: NativeCallable.listener(handler) → nativeFunction (C 函数指针)
  └─ helix_shim_create(api, ws, dartCb)
       └─ shim 造 CppHostVtable{ on_domain_events_batch=shim_on_batch, now_ms=固定值, ctx=shim }
            └─ helix_create_with_network(...) → HelixHandle*

点击发送: Dart helix_shim_command(name,payload) → shim → helix_command(...)（非阻塞 tick_tx.send）

事件回流（pump 线程）:
  cdylib → vtable.on_domain_events_batch(ctx, ptr, len)  == shim_on_batch
    └─ shim malloc(len) + memcpy(ptr..ptr+len) 到稳定堆       ← ★不变量①稳定化
    └─ dartCb(copyPtr, len)  (listener: 异步投递, 立即返回, noexcept)  ← ★②整批一次 + noexcept
Dart 事件循环: handler(copyPtr, len)
    └─ Uint8List.fromList(p.asTypedList(len)) 拷出
    └─ helix_shim_free_batch(copyPtr)   (与 shim malloc 配对, 无状态 free)  ← ★③free 无状态
    └─ decode_batch（u32 count + 每条 [u32 len, bytes] LE）→ setState → ListView 出现 im:post:sending
```

### 跨边界三大不变量（与 helix 契约一致）

- **①稳定化**：helix vtable 契约规定回调里 `ptr` **仅回调期间有效**；`.listener` 异步投递，Dart handler 执行时 pump buffer 已失效 → shim 必须在 `shim_on_batch` 里 `malloc`+`memcpy` 到稳定堆，把稳定指针交给 Dart。
- **②整批一次 + noexcept**（HX-C007 + HX-C006）：shim **整批一块**一次性传 Dart，**禁** per-event 多次调 listener（逐条跨边界 = 违 HX-C007）；回调体 `noexcept` + try/catch 兜底，C++ 异常绝不外逸跨 `extern "C"`。
- **③free_batch 无状态**：`helix_shim_free_batch` 只 `free(ptr)`，**绝不解引用 shim/handle** → 「join 后 handle 已 destroy、Dart 仍调 free_batch」**不是 UAF**（内存独立于 handle）；全程 C `malloc/free`，不混 C++ `new`。

---

## ptr 生命周期 / 销毁 6 步（唯一同步屏障 = join pump 线程）

```
① shim 停发命令
② helix_shim_destroy → helix_destroy: drop tick_tx → send shutdown → join pump_thread
   （join 只锚「Rust→C++ 同步回调已返回」，不锚「Dart 异步队列已空」）
③ Rust 不再触达 ctx
④ Dart drain port queue（await 一拍）处理残留批（含 free_batch）
⑤ _cb.close()
⑥ shim 释放 ctx / 自身
```

> 末批语义：`_cb.close()` 前 drain Dart port queue 避免末批 leak；冒烟显式**接受丢末批**（不阻塞）。

---

## shim 扁平 ABI（Dart FFI 绑定面）

```c
// helix_shim.h — len 统一 uintptr_t 与 helix vtable 一致
typedef void (*HelixDartBatchCb)(const uint8_t* ptr, uintptr_t len);

void*    helix_shim_create(const char* api, const char* ws, HelixDartBatchCb cb);
int32_t  helix_shim_command(void* shim, const char* name, const char* payload);
void     helix_shim_free_batch(const uint8_t* ptr);  // 仅释放 shim_on_batch malloc 的批, 非 vtable ptr
void     helix_shim_destroy(void* shim);
```

Dart 侧签名对齐：`NativeCallable<Void Function(Pointer<Uint8>, UintPtr)>`（`UintPtr`，非 `IntPtr`）。
批 wire = `batch.rs` 既有格式（`u32 count` + 每条 `u32 len + bytes` LE），shim/cdylib 无需新导出。

---

## 当前 C 路线阶段状态

| 阶段 | 内容 | 状态 |
|---|---|---|
| **阶段 1**（不依赖 Xcode）| C++ shim + Dart FFI 绑定 + `build_native.sh`（`cargo build --release` + `clang`）+ `dart run` 纯 console dart:ffi→shim→cdylib→listener 异步回调证完整链路 + UI/integration_test/maestro 骨架代码就绪 + 本仓三文档 | **done**（console 冒烟 = 链路真证，仅缺 Flutter widget 渲染层）|
| **阶段 2**（待用户装完整 Xcode + CocoaPods）| `flutter test integration_test -d macos` 跑绿 | **BLOCKED-on-Xcode**（实测：完整 Xcode + CocoaPods 未装 → `flutter run/test -d macos` 暂不可运行）|

> helix 仓零回归护栏：`cargo check -p helix-driver-ffi` + `bash scripts/cpp-smoke.sh` + `scripts/structure-gate.sh` 必绿。详见 spec §11 / §13。
