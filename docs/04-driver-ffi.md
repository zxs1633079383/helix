# helix-driver-ffi —— FFI 驱动层设计文档

> **版本** 2026-06-08 · C ABI 稳定层 + uniffi 双轨方案
>
> **本文目的**：详述 `helix-driver-ffi` 如何通过 C ABI（`extern "C"` / `#[no_mangle]` / `repr(C)` / opaque ptr）和 uniffi（Swift/Kotlin 绑定）把 core ports 暴露给移动端宿主，以及 host 回调如何提供 I/O 能力。

---

## 0. 定位与两轨方案

```
iOS (Swift)          Android (Kotlin/JVM)         C/C++ 宿主
    │                        │                         │
    │ uniffi bindings         │ uniffi bindings          │ extern "C"
    │ HelixCore.swift         │ HelixCore.kt             │ helix.h
    ▼                        ▼                         ▼
helix-driver-ffi  (crate-type = ["cdylib", "staticlib"])
    │  implements: Transport / Storage / Clock / EventSink / HttpRequester
    │  owns:       host callback table / 同步 SQLite (rusqlite) / C ABI 边界
    ▼
helix-core + helix-im
```

### 两轨并存

| 轨道 | 适用场景 | 生成产物 |
|---|---|---|
| **C ABI 轨**（`extern "C"` + `#[no_mangle]` + `cbindgen`）| 任何能加载 `.dylib` / `.so` / `.a` 的宿主（C++、Unity、NDK raw）| `helix.h` + `.dylib`/`.a` |
| **uniffi 轨**（`#[uniffi::export]` + UDL / proc-macro）| Swift (iOS) / Kotlin (Android) 上层 | `HelixCore.swift` + `HelixCore.kt` + 对应绑定 |

两轨共享同一份 Rust 实现，只是暴露层不同：C ABI 轨手写 `extern "C"` 函数，uniffi 轨加注解自动生成。

---

## 1. Cargo.toml

```toml
# crates/helix-driver-ffi/Cargo.toml
[package]
name    = "helix-driver-ffi"
version = "0.1.0"
edition = "2021"

[lib]
# cdylib：动态库（iOS/Android `.dylib` / `.so`）
# staticlib：静态库（iOS `.a` / ndkbuild）
crate-type = ["cdylib", "staticlib"]

[dependencies]
helix-core = { path = "../helix-core" }
bytes       = { workspace = true }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = "1"

# FFI 只在 non-wasm target
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
rusqlite   = { version = "0.31", features = ["bundled"] }
# uniffi：仅当启用 uniffi feature
uniffi     = { version = "0.27", optional = true }
# cbindgen 是构建工具，不是运行时依赖（在 build.rs 里调）

[features]
uniffi-bindings = ["dep:uniffi"]

[build-dependencies]
cbindgen = "0.27"          # 生成 C header
uniffi    = { version = "0.27", features = ["build"], optional = true }
```

---

## 2. C ABI 边界设计原则

### 2.1 ABI 稳定三要素

```rust
// ① repr(C)：确保内存布局与 C 完全一致
#[repr(C)]
pub struct HelixBytes {
    ptr: *const u8,
    len: usize,
}

// ② #[no_mangle]：防止 Rust mangling，保证 C 端能 dlsym 到名字
// ③ extern "C"：C 调用约定（cdecl），跨语言兼容
#[no_mangle]
pub extern "C" fn helix_send_command(
    handle: *mut HelixHandle,
    cmd_ptr: *const u8,
    cmd_len: usize,
) -> i32 { /* ... */ }
```

### 2.2 Opaque Pointer 模式（隐藏 Rust 内部结构）

C 端**不知道 `HelixHandle` 的内部结构**，只拿一个不透明指针。所有操作通过函数调用完成：

```c
// helix.h（cbindgen 生成）
typedef struct HelixHandle HelixHandle;  // 不透明类型，无字段定义

HelixHandle* helix_create(
    const char* ws_url,
    const char* http_base_url,
    HelixCallbacks callbacks
);
void  helix_destroy(HelixHandle* handle);
int32_t helix_send_command(HelixHandle* handle, const uint8_t* cmd, uintptr_t len);
```

Rust 侧用 `Box::into_raw` / `Box::from_raw` 管理堆上的 `HelixHandle`：

```rust
pub struct HelixHandle {
    engine: std::sync::Arc<std::sync::Mutex<helix_core::CoreEngine>>,
    rt: tokio::runtime::Handle,  // FFI 宿主可能没有 tokio runtime，driver 自建
    // ... 其他 driver 状态
}

#[no_mangle]
pub extern "C" fn helix_create(
    ws_url: *const std::ffi::CStr,
    http_base: *const std::ffi::CStr,
    callbacks: HelixCallbacks,
) -> *mut HelixHandle {
    // 安全：调用方保证指针有效且 null-terminated
    let ws_url = unsafe { (*ws_url).to_str().expect("utf8") }.to_string();
    let http_base = unsafe { (*http_base).to_str().expect("utf8") }.to_string();

    let handle = Box::new(HelixHandle::new(ws_url, http_base, callbacks));
    Box::into_raw(handle)  // 转移所有权给 C 端
}

#[no_mangle]
pub extern "C" fn helix_destroy(handle: *mut HelixHandle) {
    if handle.is_null() { return; }
    // 安全：所有权由 helix_create 转移，helix_destroy 唯一回收者
    let _ = unsafe { Box::from_raw(handle) };  // Box drop → HelixHandle drop
}
```

**所有权规则（必须严守）**：

- `helix_create` 返回的指针，C 端作为不透明 handle 传递，**不得 free**。
- `helix_destroy` 是唯一回收入口，必须调用一次且仅一次。
- C 侧 buffer 传入 Rust（如 `cmd_ptr`）：Rust 必须在函数返回前完成拷贝，不得持有指针到函数返回之后。

---

## 3. Host 回调表：C 端提供 I/O 能力

FFI 的 I/O 能力由宿主（Swift/Kotlin/C++）注入回调函数，Rust driver 通过回调执行实际 I/O：

```rust
// helix-driver-ffi/src/callbacks.rs

/// C 宿主注入的回调表（ABI 稳定）
/// 所有回调必须是线程安全的（可能从 Rust tokio 线程调用）
#[repr(C)]
pub struct HelixCallbacks {
    /// WebSocket 发送：host 把帧发到真实 socket
    pub ws_send: Option<extern "C" fn(
        ctx: *mut std::ffi::c_void,
        data: *const u8,
        len: usize,
    ) -> i32>,

    /// 事件回调：host 消费 DomainEvent（序列化为 JSON）
    pub on_event: Option<extern "C" fn(
        ctx: *mut std::ffi::c_void,
        channel: *const std::ffi::c_char,
        payload_json: *const std::ffi::c_char,
    )>,

    /// HTTP 请求：host 执行（可能是 URLSession / OkHttp）
    /// 异步：请求完成后 host 调 helix_http_reply 把结果喂回
    pub http_request: Option<extern "C" fn(
        ctx: *mut std::ffi::c_void,
        correlation_id: u64,
        method: *const std::ffi::c_char,
        url: *const std::ffi::c_char,
        body_ptr: *const u8,
        body_len: usize,
    )>,

    /// 用户自定义 context（host 自己管理，Rust 不解释）
    pub ctx: *mut std::ffi::c_void,
}

// 安全标注：C 端保证 ctx 在 HelixHandle 生命周期内有效且不被并发修改
// （这个约束无法用类型系统表达，必须通过文档约定）
unsafe impl Send for HelixCallbacks {}
unsafe impl Sync for HelixCallbacks {}
```

### 3.1 WS 入站：host → Rust

C/Swift/Kotlin 收到 WS 帧时，调 Rust 的入站接口：

```rust
/// host 调此函数把 WS 入站帧推给 Rust engine
#[no_mangle]
pub extern "C" fn helix_ws_received(
    handle: *mut HelixHandle,
    data: *const u8,
    len: usize,
) {
    let handle = unsafe { &*handle };
    // 安全：data 必须在本函数返回前有效；立即拷贝
    let bytes = bytes::Bytes::copy_from_slice(
        unsafe { std::slice::from_raw_parts(data, len) }
    );
    // 把帧作为 Tick::Inbound 喂给 engine（通过 channel，不阻塞 C 侧调用）
    let _ = handle.engine_tx.send(helix_core::engine::Tick::Inbound(bytes));
}
```

**关键**：`data` 是 C 端 buffer，**必须在函数返回前拷贝完成**（`Bytes::copy_from_slice`），不得让 Rust 持有 C 端 buffer 指针到函数返回后。

### 3.2 HTTP 异步回报：host → Rust

HTTP 是异步操作，host 执行完后通过 `helix_http_reply` 把结果喂回：

```rust
/// host HTTP 请求完成后调此函数
#[no_mangle]
pub extern "C" fn helix_http_reply(
    handle: *mut HelixHandle,
    correlation_id: u64,
    status: u16,
    body_ptr: *const u8,
    body_len: usize,
) {
    let handle = unsafe { &*handle };
    let body = bytes::Bytes::copy_from_slice(
        unsafe { std::slice::from_raw_parts(body_ptr, body_len) }
    );
    let _ = handle.engine_tx.send(helix_core::engine::Tick::HttpReply {
        correlation_id,
        result: Ok(helix_core::effect::HttpResponse { status, body }),
    });
}
```

---

## 4. Transport Port 实现：`FfiTransport`

```rust
// helix-driver-ffi/src/transport.rs
use helix_core::ports::Transport;
use bytes::Bytes;

pub struct FfiTransport {
    /// WS 入站帧（由 helix_ws_received 推入）
    inbound_rx: futures_channel::mpsc::Receiver<Bytes>,
    /// WS 出站 → 调 host 回调
    callbacks: std::sync::Arc<HelixCallbacks>,
}

#[async_trait::async_trait]
impl Transport for FfiTransport {
    type Error = FfiError;

    async fn send(&mut self, frame: Bytes) -> Result<(), FfiError> {
        let callbacks = &self.callbacks;
        if let Some(ws_send) = callbacks.ws_send {
            let result = ws_send(callbacks.ctx, frame.as_ptr(), frame.len());
            if result == 0 { Ok(()) } else { Err(FfiError::WsSendFailed(result)) }
        } else {
            Err(FfiError::NoCallback("ws_send"))
        }
    }

    async fn recv(&mut self) -> Result<Bytes, FfiError> {
        use futures_util::StreamExt;
        self.inbound_rx.next().await.ok_or(FfiError::Closed)
    }

    async fn close(&mut self) -> Result<(), FfiError> {
        self.inbound_rx.close();
        Ok(())
    }
}
```

---

## 5. Storage Port 实现：`FfiStorage`（rusqlite 同步 + spawn_blocking）

FFI 端的 SQLite 通常是宿主 app 已有的 SQLite 实例，或 driver 自持 rusqlite。两者都是同步 API，用 `tokio::task::spawn_blocking` 包装：

```rust
// helix-driver-ffi/src/storage.rs
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};
use helix_core::ports::{Storage, SqlParam, Row};

/// 单写者：Arc<Mutex<Connection>>（rusqlite 非 Sync，必须 Mutex 保护）
#[derive(Clone)]
pub struct FfiStorage {
    /// WAL 模式，单连接（FFI 场景通常无高并发读，简化为单连接）
    conn: Arc<Mutex<Connection>>,
}

impl FfiStorage {
    pub fn open(path: &str) -> Result<Self, FfiError> {
        let conn = Connection::open(path)?;
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA busy_timeout=5000;
        ")?;
        Ok(FfiStorage { conn: Arc::new(Mutex::new(conn)) })
    }
}

#[async_trait::async_trait]
impl Storage for FfiStorage {
    type Error = FfiError;

    async fn execute(&self, sql: &str, params: &[SqlParam]) -> Result<(), FfiError> {
        let conn = self.conn.clone();
        let sql = sql.to_string();
        let params: Vec<SqlParam> = params.to_vec();
        // rusqlite 是同步阻塞，包进 spawn_blocking 避免阻塞 async runtime
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FfiError::LockPoisoned)?;
            let mut stmt = conn.prepare_cached(&sql)
                .map_err(FfiError::Sqlite)?;
            stmt.execute(params_from_slice(&params))
                .map(|_| ())
                .map_err(FfiError::Sqlite)
        }).await.map_err(|_| FfiError::SpawnBlocking)?
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> Result<Vec<Row>, FfiError> {
        let conn = self.conn.clone();
        let sql = sql.to_string();
        let params: Vec<SqlParam> = params.to_vec();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|_| FfiError::LockPoisoned)?;
            let mut stmt = conn.prepare_cached(&sql)
                .map_err(FfiError::Sqlite)?;
            let rows = stmt.query_map(params_from_slice(&params), |row| {
                Ok(row_from_rusqlite(row))
            }).map_err(FfiError::Sqlite)?
            .filter_map(|r| r.ok())
            .collect();
            Ok(rows)
        }).await.map_err(|_| FfiError::SpawnBlocking)?
    }

    async fn query_one(&self, sql: &str, params: &[SqlParam]) -> Result<Option<Row>, FfiError> {
        let rows = self.query(sql, params).await?;
        Ok(rows.into_iter().next())
    }
}
```

**为何 FFI 侧用 rusqlite 而非 sqlx**：

- `sqlx` 在移动端的交叉编译工具链上偶有依赖问题（`pkg-config`，`openssl`）；`rusqlite` 带 `bundled` feature 自包含 SQLite 源码，交叉编译更稳定。
- FFI 场景通常是宿主 app（iOS/Android）管理 SQLite 文件，不需要 sqlx 的 compile-time 查询检查。

---

## 6. Clock Port 实现：`FfiClock`

```rust
// helix-driver-ffi/src/clock.rs
use helix_core::ports::Clock;

/// FFI 端：host 注入 now_ms 函数指针（可选），否则用 SystemTime
#[derive(Clone)]
pub struct FfiClock {
    now_fn: Option<extern "C" fn() -> u64>,
}

impl FfiClock {
    pub fn with_system_time() -> Self {
        FfiClock { now_fn: None }
    }

    pub fn with_host_fn(f: extern "C" fn() -> u64) -> Self {
        FfiClock { now_fn: Some(f) }
    }
}

impl Clock for FfiClock {
    fn now_ms(&self) -> u64 {
        if let Some(f) = self.now_fn {
            f()
        } else {
            std::time::SystemTime::UNIX_EPOCH
                .elapsed()
                .expect("system clock before epoch")
                .as_millis() as u64
        }
    }
}
```

---

## 7. EventSink Port 实现：`FfiEventSink`

```rust
// helix-driver-ffi/src/event_sink.rs
use helix_core::ports::EventSink;
use helix_core::events::DomainEvent;
use std::ffi::CString;

#[derive(Clone)]
pub struct FfiEventSink {
    callbacks: std::sync::Arc<HelixCallbacks>,
}

impl EventSink for FfiEventSink {
    fn emit(&self, event: DomainEvent) {
        if let Some(on_event) = self.callbacks.on_event {
            if let Some((channel, payload)) = domain_event_to_wire(&event) {
                if let (Ok(c_channel), Ok(c_payload)) = (
                    CString::new(channel),
                    CString::new(payload.to_string()),
                ) {
                    on_event(
                        self.callbacks.ctx,
                        c_channel.as_ptr(),
                        c_payload.as_ptr(),
                    );
                    // CString 在此处 drop，C 侧必须在 on_event 返回前完成拷贝
                }
            }
        }
    }
}
```

**生命周期约束**：`on_event` 回调内 `channel` 和 `payload_json` 指针只在回调执行期间有效（`CString` 在 Rust 侧 `on_event` 返回后 drop）。iOS/Android host 侧必须在回调内立即拷贝字符串，不能持有指针到回调返回后。

---

## 8. uniffi 轨：Swift / Kotlin 绑定

### 8.1 为什么加 uniffi

C ABI 对 Swift/Kotlin 需要手写桥接（`UnsafePointer<UInt8>`，Kotlin FFI），容易出错。`uniffi` 自动生成类型安全的 Swift/Kotlin API，内部还是 C ABI，但上层是正常的面向对象 API。

### 8.2 UDL 定义（或 proc-macro）

```udl
// helix-driver-ffi/src/helix.udl
namespace helix {
    HelixHandle create(string ws_url, string http_base_url, HelixCallbacksUdl callbacks);
};

interface HelixHandle {
    [Throws=HelixError]
    void send_command(string cmd_json);

    void destroy();
};

/// uniffi 不直接支持函数指针，用 trait 替代
callback interface HelixCallbacksUdl {
    void on_event(string channel, string payload_json);
    [Throws=HelixError]
    void ws_send(bytes frame);
    void http_request(u64 correlation_id, string method, string url, bytes? body);
};
```

使用 proc-macro 方式（uniffi 0.25+）更简洁，不需要单独的 `.udl` 文件：

```rust
#[uniffi::export]
pub fn helix_create_uniffi(
    ws_url: String,
    http_base: String,
    callbacks: Box<dyn HelixCallbacksUdl>,
) -> Arc<HelixHandleUniffi> {
    Arc::new(HelixHandleUniffi::new(ws_url, http_base, callbacks))
}

#[uniffi::export]
impl HelixHandleUniffi {
    pub fn send_command(&self, cmd_json: String) -> Result<(), HelixError> {
        // ...
    }
}

#[uniffi::export(callback_interface)]
pub trait HelixCallbacksUdl: Send + Sync {
    fn on_event(&self, channel: String, payload_json: String);
    fn ws_send(&self, frame: Vec<u8>) -> Result<(), HelixError>;
    fn http_request(&self, correlation_id: u64, method: String, url: String, body: Option<Vec<u8>>);
}
```

生成 Swift：

```bash
# build.rs 调用或 Makefile
uniffi-bindgen generate crates/helix-driver-ffi/src/helix.udl \
    --language swift \
    --out-dir bindings/swift/

uniffi-bindgen generate crates/helix-driver-ffi/src/helix.udl \
    --language kotlin \
    --out-dir bindings/kotlin/
```

生成 C header（cbindgen）：

```rust
// build.rs
fn main() {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    cbindgen::generate(cbindgen::Config::default(), &crate_dir)
        .expect("cbindgen failed")
        .write_to_file("helix.h");

    // uniffi build stubs
    #[cfg(feature = "uniffi-bindings")]
    uniffi::generate_scaffolding("src/helix.udl").unwrap();
}
```

---

## 9. byte-in/byte-out vs handle API

| API 风格 | 适用场景 | 优势 | 劣势 |
|---|---|---|---|
| **Byte-in/byte-out**（所有通信走 `*const u8 + usize`）| C/C++ 宿主、Unity、NDK raw | 最通用、最轻量、无类型依赖 | host 需手工序列化/反序列化 |
| **Handle API**（opaque `HelixHandle*` + 命名方法）| Swift uniffi / Kotlin uniffi | 类型安全、host 代码清晰 | 依赖 uniffi 工具链 |
| **Struct 参数**（`repr(C)` struct 作参数）| 高频小消息（如 Timer fire）| 零拷贝某些字段 | ABI 演化时字段顺序冻结 |

**Helix FFI 推荐**：

- **命令入口**：`helix_send_command(handle, json_ptr, json_len)` —— 命令体积小、类型多，JSON 字符串最灵活。
- **WS 帧**：`helix_ws_received(handle, data, len)` —— 纯字节，不反序列化，driver 层处理。
- **事件出口**：`on_event(ctx, channel_cstr, json_cstr)` —— channel + JSON payload，宿主按需解析。
- **HTTP**：回调传 correlation_id + url + body bytes；回报传 correlation_id + status + body bytes。

---

## 10. Port 实现汇总

| core Port | FFI 实现类型 | 核心依赖 | 所有权规则 |
|---|---|---|---|
| `Transport` | `FfiTransport` | host `ws_send` callback + `helix_ws_received` 入口 | 入站帧立即 `copy_from_slice`；出站调回调前 buffer 在 Rust 栈上 |
| `Storage` | `FfiStorage` | `rusqlite` bundled + `spawn_blocking` | `Arc<Mutex<Connection>>` 单写者 |
| `Clock` | `FfiClock` | host `now_fn` 或 `SystemTime` | 无状态 Clone |
| `EventSink` | `FfiEventSink` | host `on_event` callback | CString 在回调返回后 drop，host 必须在回调内拷贝 |
| `HttpRequester` | `FfiHttpRequester` | host `http_request` callback + `helix_http_reply` 入口 | correlation_id 配对；body bytes 立即拷贝 |
| `IdGen` | `FfiIdGen` | host `next_id_fn` 或 uuid | 无状态 Clone |

---

## 11. 踩坑备档

1. **`repr(C)` 顺序冻结**：一旦发布 ABI，`repr(C)` struct 的字段顺序和类型永远不能改（改了 = ABI 破坏）。新增字段只能在末尾，且要版本 flag。
2. **CString 生命周期**：`on_event` 内的 `channel` / `payload_json` 指针只在回调执行期间有效，Rust drop `CString` 在回调返回后。iOS/Kotlin 侧必须 `String(cString:)` 拷贝，不能持有 `UnsafePointer`。
3. **WS 入站 buffer 立即拷贝**：`helix_ws_received` 的 `data` 指针是宿主 buffer，**必须在函数返回前完成 `Bytes::copy_from_slice`**，绝不能把指针存起来异步使用。
4. **spawn_blocking 在 tokio runtime 里**：`FfiStorage::execute/query` 调 `tokio::task::spawn_blocking`，要求 driver 已在 tokio runtime 上下文里运行。若宿主没有 tokio，driver 需自建 `tokio::runtime::Runtime` 并在 `HelixHandle::new` 里 `block_on` 或 `spawn`。
5. **`unsafe impl Send for HelixCallbacks`**：标注 Send 意味着断言 `ctx` 可以跨线程安全访问。这个断言必须靠 host 文档保证（Rust 无法检查 C 指针的线程安全性）——文档必须写清楚。
6. **rusqlite bundled vs 宿主 SQLite**：iOS/Android 系统 SQLite 版本可能过旧（iOS 14 = SQLite 3.36）。`rusqlite` bundled 自带最新版，推荐用以避免 WAL2 / UPSERT 功能缺失。代价是增加 binary size 约 500KB。
