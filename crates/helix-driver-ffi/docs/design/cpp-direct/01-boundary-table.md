# C++ 直连模型 — 最终边界面表

> FFI 边界面（pump 已实装，commit d8c3093..630a713）。
> 所有函数签名与 `helix.hpp` 同步（cbindgen 自动生成，以 .hpp 为准）。

---

## 类型

| 类型 | 性质 | 说明 |
|---|---|---|
| `HelixHandle` | **Opaque（C++ 不透明前向声明）** | 由任一 create 入口分配，`helix_destroy` 释放；Box::into_raw / from_raw 单一 owner |
| `CppHostVtable` | `repr(C)` struct（C++ 可直接构造）| C++ host 回调表；字段见下表 |

### CppHostVtable 字段

| 字段 | 类型 | 何时调用 | 状态 |
|---|---|---|---|
| `on_domain_events_batch` | `fn(ctx, ptr, len)` | pump 吐 `Effect::Emit` 批后整批调用（HX-C007）| ✅ 已实装（`FfiEventSink` + `BatchSink::flush` 每 step 整批回调）|
| `now_ms` | `fn(ctx) -> u64` | pump 每次 `step()` 前注入壁钟（HX-C002 确定性）| ✅ 已实装（`FfiClock` 调用；None 时 fallback SystemTime + warn）|
| `ctx` | `*mut void` | 原样透传给两个回调 | Rust 不解引用 |

---

## 导出函数

| 函数 | 返回 | 状态 | 说明 |
|---|---|---|---|
| `helix_create_with_endpoints(go_api, go_len, java_api, java_len, ws_url, ws_len, vtable)` | `*mut HelixHandle` / null | ✅ 推荐 | 前端分别注入 Go IM base 与 Java/default base；二者可为不同 origin；Java 为空 fail-closed；起 pump 线程 + tick channel |
| `helix_create_with_network(go_api, go_len, ws_url, ws_len, vtable)` | `*mut HelixHandle` / null | ✅ 兼容 | 保留原 ABI，Java/default base 为空且不从 Go 猜测；仅适合不依赖 Java 的 Go/WS 能力 |
| `helix_destroy(handle)` | void | ✅ 已实现 | null-check → Box::from_raw → take/drop tick_tx + send shutdown → **join pump_thread**（最后一批回调在 ctx 释放前返回）|
| `helix_set_header(handle, name, nlen, value, vlen)` | i32 | ✅ 已实现 | `headers.set_header_sync` |
| `helix_remove_header(handle, name, nlen)` | i32 | ✅ 已实现 | `headers.remove_header_sync` |
| `helix_invoke_action(handle, envelope, len)` | i32 | ✅ 已实现 | 校验版本化 action envelope 后入同一 `Tick::Command` pump |
| `helix_command(handle, name, nlen, payload, plen)` | i32 | ✅ 已实现（**有真实 pump 消费者**：`tick_rx` → step → Effect）| 立即拷贝（ACL-3）→ `Tick::Command` → `try_send`；Full=FFI_BUSY，禁静默丢 |
| `helix_query(handle, qname, qnlen, qpayload, qplen, out_ptr, out_len)` | i32 | ✅ 元信息查询 | 支持 `helix_capabilities` / `helix_version`；业务查询返回 `FFI_UNIMPLEMENTED`，必须改走 `helix_command` + DomainEvent 批次 |
| `helix_query_free(ptr, len)` | void | ✅ 已实现（null-check + Box::from_raw slice）| 配对 helix_query 成功时的 Box::into_raw |

---

## 返回码

| 常量 | 值 | 含义 |
|---|---|---|
| `FFI_OK` | 0 | 成功 |
| `FFI_NULL` | -1 | handle / 指针为 null，或 channel 已关闭 |
| `FFI_UTF8` | -2 | 字节非合法 UTF-8 |
| `FFI_INVALID` | -3 | 参数取值无效 |
| `FFI_BUSY` | -4 | tick channel 满，C++ 侧应稍后重试 |
| `FFI_UNIMPLEMENTED` | -5 | 功能 stub，M3-F4 接入后填充 |

---

## 内存契约（HX-C006）

1. **opaque pointer 单 owner**：任一 create 入口产生，`helix_destroy` 唯一释放（Box::from_raw 单次）。
2. **输入 buffer 立即拷贝（ACL-3）**：所有 `*const u8` 参数在函数返回前拷贝到 Rust 堆，C++ 侧可立即复用 buffer。
3. **`helix_query` 输出配对释放**：成功时 out_ptr 指向 Box::into_raw 分配；C++ 必须调用 `helix_query_free`，禁 free()/delete[]。
4. **panic = abort**（release profile）：panic 不跨 FFI unwind（workspace Cargo.toml `[profile.release] panic = "abort"`）。
5. **ctx 生命周期**：C++ 不得在 `helix_destroy` 返回前释放 vtable.ctx（pump join 前 on_domain_events_batch 仍可能调用）。

---

## 真实契约纠错（round-2 spec 要求记录）

| 误解 | 真实情况 |
|---|---|
| HTTP 失败响应（4xx/5xx）走 Err 路径 | 完成响应（任意 HTTP status）→ `PortOutcome::Ok(ReplyBytes(json{status,...}))`；只有传输级错误（超时/连接断）→ `Err` via `classify_port_error` |
| `PortError` 在 `tick.rs` | `tick.rs` 的 `PortError` 是 tick-level（Timeout/Http(u16)/Storage(u32)/Network/Other(u32)）；`error.rs` 的 `PortError` 才是 port trait 统一返回类型（String 承载），两者不同 |
| `step()` 签名含 `EffectSink` 参数 | 真实签名：`step(&mut self, tick: Tick, now_ms: u64) -> Result<&[Effect], CoreError>`（EffectSink 在 shell 内部；返回借用的 scratch 切片）|
