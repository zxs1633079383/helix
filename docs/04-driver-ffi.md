# helix-driver-ffi —— FFI 驱动层设计文档

> **版本** 2026-06-08 · C ABI 稳定层 + uniffi 双轨方案
> **状态更新（2026-06-15 / ADR-006/ADR-007/ADR-008）**：手机端确定为 Flutter。`helix-driver-ffi` 是当前移动端主线；Dart/Flutter 只负责 UI、路径配置、命令入口与事件订阅，不写 IM 同步逻辑、不写消息 SQL、不直接实现业务 HTTP/WS。SQLite schema/migration、StorageOp 兑现、rows reply 编解码、HTTP client 与 WebSocket client 统一复用 `helix-driver-host`；网页端 wasm 已暂停维护。
>
> **本文目的**：详述 `helix-driver-ffi` 如何通过 C ABI（`extern "C"` / `#[no_mangle]` / `repr(C)` / opaque ptr）和 uniffi（Swift/Kotlin 绑定）把 Flutter/移动端接到 Rust host driver；Flutter 主线由 Rust 侧全包 I/O，host 回调只保留事件/生命周期/可选宿主能力。登录/鉴权不归 Helix，Helix 只消费外部传入的 header/token/session 配置。

---

## 0. 定位与两轨方案

```
Flutter / Dart FFI        iOS/Android 原生宿主        C/C++ 宿主
    │                        │                         │
    │ dart:ffi               │ uniffi/C ABI              │ extern "C"
    │ package bridge         │ Swift/Kotlin wrapper      │ helix.h
    ▼                        ▼                         ▼
helix-driver-ffi  (crate-type = ["cdylib", "staticlib"])
    │  implements: Flutter C ABI / opaque handle / Dart event callback
    │  reuses:     helix-driver-host Storage + HTTP/WS client + StorageOp execution
    │  owns:       lifecycle / callback boundary / event stream bridge
    ▼
helix-core + helix-im
```

### 两轨并存

| 轨道 | 适用场景 | 生成产物 |
|---|---|---|
| **C ABI 轨**（`extern "C"` + `#[no_mangle]` + `cbindgen`）| Flutter `dart:ffi` 主线，以及任何能加载 `.dylib` / `.so` / `.a` 的宿主 | `helix.h` + `.dylib`/`.so`/`.a` |
| **uniffi 轨**（`#[uniffi::export]` + UDL / proc-macro）| 未来 Swift (iOS) / Kotlin (Android) 原生上层 | `HelixCore.swift` + `HelixCore.kt` + 对应绑定 |

两轨共享同一份 Rust 实现，只是暴露层不同：Flutter 当前优先走 C ABI；uniffi 是未来原生壳备选，不作为 Flutter MVP 阻塞项。

### 0.1 Flutter 最小 API 面

Flutter 侧 API 要薄：Dart 不理解 `Tick` / `Effect` / `StorageOp`，只调用命令并订阅事件。

```dart
final helix = Helix.open(dbPath: path, apiBaseUrl: api, wsUrl: ws);
helix.events.listen(handleImEvent);
helix.sendMessage(channelId: id, temporaryId: tmp, text: text);
helix.destroy();
```

Rust FFI 侧对应少量稳定入口：`helix_create/open`（传入 DB 路径、`api_base_url`、`ws_url`、初始 headers 等 `HostNetworkConfig`）/ `helix_start` / `helix_send_command` / `helix_set_header` / `helix_remove_header` / `helix_destroy`。消息 schema、sync、cursor、pending send、HTTP 请求执行、WS 长连收发全部留在 `helix-im` 和 shared host driver 内部。

### 0.2 网络初始化材料由宿主提供

Flutter/Dart 不实现业务 HTTP/WS，但必须在创建 Helix 时提供 shared client 的初始化材料：

- `api_base_url`：业务模块拼 HTTP path 时使用的 API 根地址。
- `ws_url`：shared WS client 建连使用的地址。
- `headers`：Authorization、租户、设备、版本等默认 header；登录模块拿到/刷新 token 后通过 `helix_set_header` 更新。
- 可选平台配置：timeout、user-agent、proxy/TLS/cert 策略等，进入 `HostNetworkConfig`，由 shared client 消费。

业务模块不直接读取 token/header；HTTP request 和 WS connect/重连时由 shared header registry 统一注入。

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
helix-driver-host = { path = "../helix-driver-host" }
bytes       = { workspace = true }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = "1"

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

## 3. Host 回调表：C 端提供事件与可选宿主能力

Flutter 主线选择 Rust 侧全包 I/O：Dart 不执行 IM sync、消息 SQL、cursor 推进、echo 对账、业务 HTTP 请求或 WS 长连，也不理解 `Tick` / `Effect` / `StorageOp`。C 回调表主线只用于事件投递、可选时间/ID 注入和生命周期通知；header/token 更新走显式配置 API（如 `helix_set_header`），进入 shared header registry。历史的 `ws_send` / `http_request` host callback 只作为非 Flutter 宿主或平台网络栈强约束下的逃生口，不是 PC/Flutter 产品路径。

```rust
// helix-driver-ffi/src/callbacks.rs

/// C 宿主注入的回调表（ABI 稳定）
/// 所有回调必须是线程安全的（可能从 Rust tokio 线程调用）
#[repr(C)]
pub struct HelixCallbacks {
    /// 可选逃生口：host 把帧发到真实 socket。
    /// Flutter 产品主路径为空，Rust shared WS client 负责真实连接。
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

    /// 可选逃生口：host 执行 HTTP（可能是 URLSession / OkHttp）。
    /// Flutter 产品主路径为空，Rust shared HTTP client 负责真实请求。
    /// 异步：请求完成后 host 调 helix_http_reply 把结果喂回。
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

### 3.1 可选逃生口：WS 入站 host → Rust

Flutter 产品主路径不走此入口：shared WS client 在 Rust 内部建连、收帧并回灌 Tick。只有当某个平台必须使用宿主网络栈（例如系统代理、证书、后台网络能力）时，C/Swift/Kotlin 才收到 WS 帧后调 Rust 的入站接口：

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

### 3.2 可选逃生口：HTTP 异步回报 host → Rust

Flutter 产品主路径不走此入口：shared HTTP client 在 Rust 内部执行请求并回灌 PortReply。只有 host-network 逃生口启用时，HTTP 才由宿主异步执行，完成后通过 `helix_http_reply` 把结果喂回：

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

## 4. Transport / HttpRequester Port 实现：薄壳复用 shared client

`FfiTransport` 与 `FfiHttpRequester` 不再各自实现一套移动端网络逻辑。产品主路径是：

- `FfiTransport` 持有 shared WS client handle，调用 `connect/send/recv/close`。
- `FfiHttpRequester` 持有 shared HTTP client handle，调用 `request`。
- header/token/session 由 `helix_set_header` 这类 FFI API 注入 shared header registry。
- shared client 初始化时接收 `HostNetworkConfig`，后续 HTTP 请求和 WS connect/重连都从同一个 header registry 取最新 header。
- host `ws_send` / `http_request` callback 仅作为 host-network 逃生口，不是默认路径。

逃生口示例仍保留如下，用于说明 FFI buffer 所有权约束：

```rust
// helix-driver-ffi/src/transport.rs
use helix_core::ports::Transport;
use bytes::Bytes;

/// Host-network fallback only. Flutter product path uses shared WS client.
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

## 5. Storage Port 实现：`FfiStorage`（复用 `helix-driver-host`）

`FfiStorage` 是 `helix_driver_host::HostStorage` 的薄 wrapper。Flutter/Dart 只传 DB 路径或 app data dir；Rust 打开 SQLite、执行 migration、兑现 StorageOp、回灌 PortReply。

```rust
#[derive(Clone)]
pub struct FfiStorage {
    inner: helix_driver_host::HostStorage,
}
```

共享层提供：

- SQLite WAL / synchronous=NORMAL / busy_timeout
- `_helix_monotonic(scope_key,value)` schema 与 migration
- `batch_upsert` / `batch_update` / `monotonic_upsert` / `get` / `scan`
- `StorageOp -> PortOutcome`
- `rows_to_reply_bytes` / `rows_from_reply_bytes`

FFI 层保留：

- C ABI / opaque handle / destroy 单 owner
- Dart event callback -> Stream
- Flutter 最小命令 API：`helix_create/open`、`helix_start/connect`、`helix_send_command` 或 `send_message`、必要 query、`helix_destroy`

历史草稿中的宿主 SQLite 句柄回调方案不再作为 Flutter 主线；若未来 iOS/Android 原生壳需要接宿主 SQLite，必须先保证与 `helix-driver-host` conformance tests 共享同一语义。

<!-- historical sketch retained below for context -->

FFI 端若选择宿主 SQLite 句柄，两者都是同步 API，用 `tokio::task::spawn_blocking` 包装：

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

**为何 shared host 层用 rusqlite 而非 native sqlx + ffi rusqlite 双轨**：

- `sqlx` 在移动端的交叉编译工具链上偶有依赖问题（`pkg-config`，`openssl`）；`rusqlite` bundled 自包含 SQLite 源码，交叉编译更稳定。
- native/ffi 双实现会让 messages SQL、`_helix_monotonic`、StorageOp 语义和 row 映射漂移；统一 rusqlite 后只有一份 conformance tests。

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
    // Host-network fallback only. Flutter product path uses Rust shared clients.
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
    // Host-network fallback only. Flutter product path uses Rust shared clients.
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
- **WS**：Flutter 产品主路径不暴露 WS 帧入口；shared WS client 在 Rust 内部收发并把入站帧转成 Tick。`helix_ws_received(handle, data, len)` 仅保留给 host-network 逃生口。
- **事件出口**：`on_event(ctx, channel_cstr, json_cstr)` —— channel + JSON payload，宿主按需解析。
- **HTTP**：Flutter 产品主路径不通过 host 回调执行请求；shared HTTP client 在 Rust 内部完成请求并回灌 PortReply。`http_request` / `helix_http_reply` 仅保留给 host-network 逃生口。

---

## 10. Port 实现汇总

| core Port | FFI 实现类型 | 核心依赖 | 所有权规则 |
|---|---|---|---|
| `Transport` | `FfiTransport` 薄壳 → shared WS client | `helix-driver-host` shared WS runtime；host callback 仅逃生口 | 产品主路径无 host WS buffer；逃生口入站帧必须立即 `copy_from_slice` |
| `Storage` | `FfiStorage` → `HostStorage` | `helix-driver-host` / `rusqlite` bundled + `spawn_blocking` | shared blocking SQLite worker |
| `Clock` | `FfiClock` | host `now_fn` 或 `SystemTime` | 无状态 Clone |
| `EventSink` | `FfiEventSink` | host `on_event` callback | CString 在回调返回后 drop，host 必须在回调内拷贝 |
| `HttpRequester` | `FfiHttpRequester` 薄壳 → shared HTTP client | `helix-driver-host` shared HTTP client；host callback 仅逃生口 | 产品主路径无 host HTTP buffer；逃生口 correlation_id 配对、body bytes 立即拷贝 |
| `IdGen` | `FfiIdGen` | host `next_id_fn` 或 uuid | 无状态 Clone |

---

## 11. 踩坑备档

1. **`repr(C)` 顺序冻结**：一旦发布 ABI，`repr(C)` struct 的字段顺序和类型永远不能改（改了 = ABI 破坏）。新增字段只能在末尾，且要版本 flag。
2. **CString 生命周期**：`on_event` 内的 `channel` / `payload_json` 指针只在回调执行期间有效，Rust drop `CString` 在回调返回后。iOS/Kotlin 侧必须 `String(cString:)` 拷贝，不能持有 `UnsafePointer`。
3. **WS 入站 buffer 立即拷贝**：`helix_ws_received` 的 `data` 指针是宿主 buffer，**必须在函数返回前完成 `Bytes::copy_from_slice`**，绝不能把指针存起来异步使用。
4. **spawn_blocking 在 tokio runtime 里**：`FfiStorage::execute/query` 调 `tokio::task::spawn_blocking`，要求 driver 已在 tokio runtime 上下文里运行。若宿主没有 tokio，driver 需自建 `tokio::runtime::Runtime` 并在 `HelixHandle::new` 里 `block_on` 或 `spawn`。
5. **`unsafe impl Send for HelixCallbacks`**：标注 Send 意味着断言 `ctx` 可以跨线程安全访问。这个断言必须靠 host 文档保证（Rust 无法检查 C 指针的线程安全性）——文档必须写清楚。
6. **rusqlite bundled vs 宿主 SQLite**：iOS/Android 系统 SQLite 版本可能过旧（iOS 14 = SQLite 3.36）。`rusqlite` bundled 自带最新版，推荐用以避免 WAL2 / UPSERT 功能缺失。代价是增加 binary size 约 500KB。
