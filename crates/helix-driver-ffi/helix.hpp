#pragma once

#include <cstdarg>
#include <cstdint>
#include <cstdlib>
#include <ostream>
#include <new>
struct HelixHandle;

/// 操作成功
constexpr static const int32_t FFI_OK = 0;

/// 传入 handle / 指针为 null
constexpr static const int32_t FFI_NULL = -1;

/// 字节序列不是合法 UTF-8
constexpr static const int32_t FFI_UTF8 = -2;

/// 参数取值无效（header registry 错误等）
constexpr static const int32_t FFI_INVALID = -3;

/// tick channel 满，调用方应稍后重试（M3-F4 pump 接入前此码不会实际出现）
constexpr static const int32_t FFI_BUSY = -4;

/// 功能尚未实现（本轮留 stub，M3-F4 pump 接入后填充）
constexpr static const int32_t FFI_UNIMPLEMENTED = -5;

/// C++ host 回调表。
///
/// `repr(C)` 保证 ABI 稳定，cbindgen 自动扫描到 helix.hpp。
///
/// # Safety
///
/// - 所有函数指针若非 None，在 helix_destroy 返回前必须保持有效（C++ 管理其生命周期）。
/// - `ctx` 指向的对象由 host 自行保证跨线程安全；Rust 不解引用 `ctx`，只原样回传。
struct CppHostVtable {
  /// 领域事件批回调。
  ///
  /// pump（M3-F4）在一个 step 产出的所有 `Effect::Emit` 序列化为单个 byte buffer 后，
  /// **整批一次**调用此函数（HX-C007）。本轮 pump 未建，此字段仅声明契约。
  ///
  /// - `ctx`：透传 host 自定义 context，Rust 不解引用。
  /// - `ptr`：指向序列化后的 DomainEventBytes 批数据；仅在本次回调期间有效。
  /// - `len`：批数据字节长度。
  ///
  /// # Safety（C++ 侧）
  ///
  /// 函数必须在返回前拷贝或消费 `ptr..ptr+len` 的数据，不得在回调返回后持有指针。
  void (*on_domain_events_batch)(void *ctx, const uint8_t *ptr, uintptr_t len);
  /// 壁钟时间（Unix epoch 毫秒）。
  ///
  /// pump（M3-F4）在每次 `ExecutionShell::step()` 前调用，注入确定性时间（HX-C002）。
  /// 本轮 pump 未建，此字段仅声明契约。
  uint64_t (*now_ms)(void *ctx);
  /// 用户自定义 context，Rust 不解引用、不拥有，仅原样回传给回调函数。
  void *ctx;
};

extern "C" {

/// 创建带 shared host network client 的 Helix 句柄（C++ 直连模型）。
///
/// 返回不透明指针；宿主必须持有并传给所有后续 API 调用，最终调用 `helix_destroy` 释放。
/// 失败时返回 null（字符串无效 / 网络客户端初始化失败）。
///
/// # Safety
///
/// - `api_base_url` / `ws_url`：非 null，各自指向至少对应 `_len` 字节、
///   在本函数调用期间有效的 UTF-8 内存。
/// - `vtable`：函数指针若非 None，在 `helix_destroy` 返回前必须保持有效（C++ 侧管生命周期）。
HelixHandle *helix_create_with_network(const uint8_t *api_base_url,
                                       uintptr_t api_base_url_len,
                                       const uint8_t *ws_url,
                                       uintptr_t ws_url_len,
                                       CppHostVtable vtable);

/// 销毁 Helix 引擎实例，释放所有资源。调用后 `handle` 不可再使用。
///
/// # Safety
///
/// - `handle` 必须是 `helix_create_with_network` 返回的有效指针，或 null（null 时无操作）。
/// - 调用方必须保证此后不再向 `handle` 传递给任何 helix API（独占所有权契约）。
/// - **C++ 不得在本函数返回前释放 vtable.ctx**：pump（M3-F4）在 join 完成前仍可能回调。
///   TODO(M3-F4): pump 线程建好后，在此处 join(pump_thread) 后再返回，
///   保证 on_domain_events_batch 不会在 ctx 释放后被调用。
void helix_destroy(HelixHandle *handle);

/// 更新 shared header registry。后续 HTTP 请求和 WS connect/reconnect 会读取最新值。
///
/// # Safety
///
/// - `handle`：helix_create_with_network 返回的有效指针，或 null（返回 FFI_NULL）。
/// - `name` / `value`：各自指向至少对应 `_len` 字节、在本函数调用期间有效的 UTF-8 内存。
int32_t helix_set_header(HelixHandle *handle,
                         const uint8_t *name,
                         uintptr_t name_len,
                         const uint8_t *value,
                         uintptr_t value_len);

/// 移除 shared header registry 中的 header。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name`：指向至少 `name_len` 字节有效 UTF-8 内存，在本调用期间有效。
int32_t helix_remove_header(HelixHandle *handle, const uint8_t *name, uintptr_t name_len);

/// 向 engine 发送应用命令（UI → engine）。
///
/// 立即将 `name` / `payload` 拷贝到 Rust 堆（ACL-3），构造 `Tick::Command`，
/// 并 **非阻塞** 尝试送入 tick channel：
///   - channel 满 → 返回 `FFI_BUSY`（C++ 侧应稍后重试，禁静默丢弃）。
///   - pump 未建时命令仅入队不被消费，等 M3-F4 pump 接入后自动处理。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name` / `payload`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
///   `name` 必须是合法 UTF-8；`payload` 为任意字节（不强要求 UTF-8）。
///
/// # TODO(M3-F4)
///
/// pump 接入前命令仅入队不被消费；pump 接入后 tick channel rx 端被 pump 主循环驱动。
int32_t helix_command(HelixHandle *handle,
                      const uint8_t *name,
                      uintptr_t name_len,
                      const uint8_t *payload,
                      uintptr_t payload_len);

/// 同步读取 engine 状态（stub，M3-F4 接入后填充）。
///
/// 本轮返回 `FFI_UNIMPLEMENTED`，不写 out_ptr / out_len。
///
/// # 配对约定
///
/// 成功（返回 0）时，`*out_ptr` 指向 Rust 分配的 boxed slice；
/// C++ 侧**必须**调用 `helix_query_free(*out_ptr, *out_len)` 释放，禁止 free() / delete[]。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `query_name` / `query_payload`：各自指向对应 `_len` 字节有效内存，在本调用期间有效。
/// - `out_ptr` / `out_len`：非 null，指向调用方分配的存储位置。
///
/// # TODO(M3-F4)
///
/// 实现：经 pump runtime Handle 的 `spawn_blocking` 跑 Storage 同步读
/// （HX-C006 §5 禁止在 tokio worker 直接 block_on）→ 序列化结果
/// → `into_boxed_slice()` → `Box::into_raw(slice)` 写 out_ptr / out_len（配对 helix_query_free）。
/// **禁止** 在本函数内 block_on（死锁：若 tokio 运行时在同一线程，block_on 会死锁）。
int32_t helix_query(HelixHandle *handle,
                    const uint8_t *_query_name,
                    uintptr_t _query_name_len,
                    const uint8_t *_query_payload,
                    uintptr_t _query_payload_len,
                    uint8_t **out_ptr,
                    uintptr_t *out_len);

/// 释放 `helix_query` 返回的 boxed slice。
///
/// 仅用于释放 `helix_query` 成功时通过 out_ptr/out_len 返回的内存，
/// 禁止用于任何其他 Rust 分配的内存（UB）。
///
/// # Safety
///
/// - `ptr`：必须是 `helix_query` 通过 `out_ptr` 写入的值，或 null（null 时无操作）。
/// - `len`：必须与 `helix_query` 写入 `out_len` 的值完全一致。
/// - 调用后 `ptr` 失效，不可再访问（禁 double-free）。
void helix_query_free(uint8_t *ptr,
                      uintptr_t len);

}  // extern "C"
