#pragma once

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
typedef struct HelixHandle HelixHandle;

/*
 操作成功
 */
#define FFI_OK 0

/*
 传入 handle / 指针为 null
 */
#define FFI_NULL -1

/*
 字节序列不是合法 UTF-8
 */
#define FFI_UTF8 -2

/*
 参数取值无效（header registry 错误等）
 */
#define FFI_INVALID -3

/*
 tick channel 满，调用方应稍后重试（M3-F4 pump 接入前此码不会实际出现）
 */
#define FFI_BUSY -4

/*
 功能尚未实现（本轮留 stub，M3-F4 pump 接入后填充）
 */
#define FFI_UNIMPLEMENTED -5

/*
 C++ host 回调表。

 `repr(C)` 保证 ABI 稳定，cbindgen 自动扫描到 helix.hpp。

 # Safety

 - 所有函数指针若非 None，在 helix_destroy 返回前必须保持有效（C++ 管理其生命周期）。
 - `ctx` 指向的对象由 host 自行保证跨线程安全；Rust 不解引用 `ctx`，只原样回传。
 */
typedef struct CppHostVtable {
  /*
   领域事件批回调。

   pump 在一个 step 产出的所有 `Effect::Emit` 序列化为单个 byte buffer 后，**整批一次**调用
   此函数（HX-C007，绝不逐条过边界）。

   - `ctx`：透传 host 自定义 context，Rust 不解引用。
   - `ptr`：指向序列化后的 DomainEventBytes 批数据；**仅在本次回调期间有效**（借出契约）。
   - `len`：批数据字节长度。

   ## buffer 所有权 = 借出契约（冻结，不新增 free 符号）

   `ptr..ptr+len` 由 Rust own，仅在 `cb` 回调期间有效，回调返回后 Rust 就地 drop。**这是正确
   边界，无 UAF**——不改 ownership-transfer、不新增 `helix_free_batch`（保 §1.3 七符号冻结）。
   Dart 异步如何吸收：**C++ shim 在回调线程内 `malloc + memcpy`** 到稳定堆，再投递给 Dart
   isolate（`docs/flutter-demo-pointer.md` ①稳定化）。稳定化是 shim 层的事，非 Rust 导出。

   # Safety（C++ 侧）

   函数必须在返回前拷贝或消费 `ptr..ptr+len` 的数据，不得在回调返回后持有指针。
   */
  void (*on_domain_events_batch)(void *ctx, const uint8_t *ptr, uintptr_t len);
  /*
   壁钟时间（Unix epoch 毫秒）。

   pump 在每次 `ExecutionShell::step()` 前同步调用，注入确定性时间（HX-C002 / GOAL §1.2）。
   **由 C++ shim host 同步提供**（shim 与 pump 同进程，回调线程内同步返回 epoch ms）——确定性
   注入靠此字段成立。**禁移除字段让 Rust SystemTime 成正路**（那会使 FFI 端比 native 更不确定）。
   测试用 mock now_ms 跑确定性回放。None 时 clock.rs fallback SystemTime（仅健壮性兜底 + warn）。
   */
  uint64_t (*now_ms)(void *ctx);
  /*
   用户自定义 context，Rust 不解引用、不拥有，仅原样回传给回调函数。
   */
  void *ctx;
} CppHostVtable;

/*
 提交一个严格版本化的 SurfaceRuntime action envelope（UI → engine）。

 与 `helix_command` 不同，这个 ABI 没有可伪造的 business command name：payload 必须是完整
 `ActionInvocationEnvelope`，由 helix-im 在入队前校验 schema/unknown fields，并在 module 中再与
 host 注入的 runtime actor 和 opaque lease 比对。返回 `FFI_OK` 仅代表 Tick 已入队；业务完成仍需
 等待同 requestId causation 的 `im:interaction:frame`。

 # Safety

 - `handle`：有效指针或 null。
 - `envelope`：指向至少 `envelope_len` 字节、并在本调用期间保持有效的内存；函数返回前会复制。
 */
int32_t helix_invoke_action(HelixHandle *handle,
                            const uint8_t *envelope,
                            uintptr_t envelope_len);

/*
 向 engine 发送应用命令（UI → engine）。

 立即将 `name` / `payload` 拷贝到 Rust 堆（ACL-3），构造 `Tick::Command`，
 并 **非阻塞** 尝试送入 tick channel（pump 线程 rx 端消费 → step → Effect → 批回 C++）：
   - channel 满 → 返回 `FFI_BUSY`（C++ 侧应稍后重试，禁静默丢弃）。
   - destroy 后 channel 已关闭 → 返回 `FFI_NULL`。

 # Safety

 - `handle`：有效指针或 null。
 - `name` / `payload`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
   `name` 必须是合法 UTF-8；`payload` 为任意字节（不强要求 UTF-8）。
 */
int32_t helix_command(HelixHandle *handle,
                      const uint8_t *name,
                      uintptr_t name_len,
                      const uint8_t *payload,
                      uintptr_t payload_len);

/*
 创建带 shared host network client 的 Helix 句柄（C++ 直连模型）。

 返回不透明指针；宿主必须持有并传给所有后续 API 调用，最终调用 `helix_destroy` 释放。
 失败时返回 null（字符串无效 / 网络客户端初始化失败）。

 # Safety

 - `api_base_url` / `ws_url`：非 null，各自指向至少对应 `_len` 字节、
   在本函数调用期间有效的 UTF-8 内存。
 - `vtable`：函数指针若非 None，在 `helix_destroy` 返回前必须保持有效（C++ 侧管生命周期）。
 */
HelixHandle *helix_create_with_network(const uint8_t *api_base_url,
                                       uintptr_t api_base_url_len,
                                       const uint8_t *ws_url,
                                       uintptr_t ws_url_len,
                                       struct CppHostVtable vtable);

/*
 销毁 Helix 引擎实例，释放所有资源。调用后 `handle` 不可再使用。

 ## 关闭顺序（graceful drain + join，独占所有权契约）

 1. 取出并 drop `tick_tx` → 关闭 tick channel（pump recv None 进 drain）。
 2. 取出并 send `shutdown_tx`（双触发冗余：drop tick_tx 与 shutdown 任一即触发 host graceful drain）。
 3. **join `pump_thread`**：阻塞到 pump 线程退出 → 保证最后一批 `on_domain_events_batch`
    回调已返回，C++ 之后释放 vtable.ctx 安全（不会 use-after-free）。

 # Safety

 - `handle` 必须是 `helix_create_with_network` 返回的有效指针，或 null（null 时无操作）。
 - 调用方必须保证此后不再向 `handle` 传递给任何 helix API（独占所有权契约，无 double-free）。
 - **C++ 不得在本函数返回前释放 vtable.ctx**：join 完成前 pump 仍可能回调；本函数返回即安全。
 */
void helix_destroy(HelixHandle *handle);

/*
 更新 shared header registry。后续 HTTP 请求和 WS connect/reconnect 会读取最新值。

 # Safety

 - `handle`：helix_create_with_network 返回的有效指针，或 null（返回 FFI_NULL）。
 - `name` / `value`：各自指向至少对应 `_len` 字节、在本函数调用期间有效的 UTF-8 内存。
 */
int32_t helix_set_header(HelixHandle *handle,
                         const uint8_t *name,
                         uintptr_t name_len,
                         const uint8_t *value,
                         uintptr_t value_len);

/*
 移除 shared header registry 中的 header。

 # Safety

 - `handle`：有效指针或 null。
 - `name`：指向至少 `name_len` 字节有效 UTF-8 内存，在本调用期间有效。
 */
int32_t helix_remove_header(HelixHandle *handle, const uint8_t *name, uintptr_t name_len);

/*
 同步读取 ABI 元信息。

 ## 查询边界

 元信息 query 由 FFI 直接返回：
 - `helix_capabilities`：返回移动端公开契约能力集 JSON
 - `helix_version`：返回 `{"abi":1}`

 业务 query 经 `helix_command` 注入异步 query command，由共享 `helix-im` 执行
 local-first/远端 fallback，并通过批量 DomainEvent 返回结果。

 ## 配对约定

 成功（返回 0）时，`*out_ptr` 指向 Rust 分配的 boxed slice；
 C++ 侧**必须**调用 `helix_query_free(*out_ptr, *out_len)` 释放，禁止 free() / delete[]。

 # Safety

 - `handle`：有效指针或 null。
 - `query_name`：指向对应 `_len` 字节有效内存，在本调用期间有效。
 - `query_payload`：为保持 C ABI 形状保留，当前元信息查询忽略该参数。
 - `out_ptr` / `out_len`：非 null，指向调用方分配的存储位置。
 */
int32_t helix_query(HelixHandle *handle,
                    const uint8_t *query_name,
                    uintptr_t query_name_len,
                    const uint8_t *query_payload,
                    uintptr_t query_payload_len,
                    uint8_t **out_ptr,
                    uintptr_t *out_len);

/*
 释放 `helix_query` 返回的 boxed slice。

 仅用于释放 `helix_query` 成功时通过 out_ptr/out_len 返回的内存，
 禁止用于任何其他 Rust 分配的内存（UB）。

 # Safety

 - `ptr`：必须是 `helix_query` 通过 `out_ptr` 写入的值，或 null（null 时无操作）。
 - `len`：必须与 `helix_query` 写入 `out_len` 的值完全一致。
 - 调用后 `ptr` 失效，不可再访问（禁 double-free）。
 */
void helix_query_free(uint8_t *ptr,
                      uintptr_t len);
