# helix-driver-wasm —— WASM 驱动层设计文档

> **状态更新（2026-06-15 / ADR-006）**：网页端暂不做，`helix-driver-wasm` 暂停维护。本文仅作为历史实验和未来重启网页端时的参考，不进入当前 roadmap、CI 必跑矩阵、交付验收或日常修复范围。当前产品目标是 PC Tauri（`helix-driver-native`）+ Flutter 手机端（`helix-driver-ffi`）。
>
> **版本** 2026-06-08 · 基于三栖约束底座 + wasm32-unknown-unknown 编译约束
>
> **本文目的**：详述 `helix-driver-wasm` 如何用 wasm-bindgen-futures / web-sys / sqlite-wasm-rs / fetch API 实现全部 core ports，严守 wasm32-unknown-unknown 的三个硬约束（无 std::thread / 无 Instant / getrandom js feature）。

---

## 0. 定位与编译约束

```
浏览器 JS 宿主 (Angular / Vanilla)
     │  wasm-bindgen 胶水层
     ▼
helix-driver-wasm  (crate-type = ["cdylib"])
     │  implements: Transport / Storage / Clock / EventSink / HttpRequester
     │  owns:       web-sys WebSocket / OPFS sahpool / performance.now / fetch
     │  spawn:      wasm_bindgen_futures::spawn_local (单线程!)
     ▼
helix-core + helix-im  (rlib, 无 I/O)
```

**三个绝对约束（wasm32-unknown-unknown）**：

| 约束 | 原因 | 对应设计 |
|---|---|---|
| 无 `std::thread::spawn` | wasm 单线程，无 OS 线程 | 所有并发靠 `spawn_local` (JS Promise chain) |
| 无 `std::time::Instant` | 编译期 panic "not supported on wasm" | Clock port 用 `js_sys::Date::now()` |
| `getrandom` 必须 `features = ["js"]` | 否则 link 阶段 `__getrandom_custom` undefined | workspace Cargo.toml 级声明 |

---

## 1. Cargo.toml

```toml
# crates/helix-driver-wasm/Cargo.toml
[package]
name     = "helix-driver-wasm"
version  = "0.1.0"
edition  = "2021"

# WASM 必须输出 cdylib（被 wasm-bindgen 工具链打包）
[lib]
crate-type = ["cdylib"]

[dependencies]
helix-core  = { path = "../helix-core" }
bytes       = { workspace = true }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = "1"

# ---- 仅 wasm32 target ----
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen          = "0.2"
wasm-bindgen-futures  = "0.4"
js-sys                = "0.3"
web-sys = { version = "0.3", features = [
    "WebSocket",
    "MessageEvent",
    "ErrorEvent",
    "CloseEvent",
    "BinaryType",
    "Window",
    "Performance",
    "WorkerGlobalScope",
    "Request",
    "RequestInit",
    "RequestMode",
    "Response",
    "Headers",
    "Blob",
]}
gloo-timers  = { version = "0.3", features = ["futures"] }
gloo-net     = { version = "0.6", features = ["websocket", "http"] }
futures-channel = { workspace = true }
# sqlite-wasm-rs: OPFS-based SQLite for wasm
sqlite-wasm-rs = { version = "0.3", features = ["opfs"] }
getrandom    = { version = "0.2", features = ["js"] }  # 必须 js feature
```

**工作区级 getrandom 声明（必须）**：

```toml
# workspace Cargo.toml [dependencies] 节
getrandom = { version = "0.2", features = ["js"] }
```

原因：任何依赖 `rand` 或 `uuid` 的 crate（包括经过 helix-core 传递的）在 wasm 链接时都要找 `getrandom`。若不在 workspace 级强制 `js` feature，会出现：

```
error: symbol `__getrandom_custom` is not defined
```

这是 wasm 编译最常见 link 失败，workspace 级一次性修复所有路径。

---

## 2. Transport Port 实现：`WasmTransport`

### 2.1 架构选型：`gloo-net` WebSocket vs 手写 `web-sys` 桥接

| 方案 | 优势 | 劣势 |
|---|---|---|
| `gloo-net::websocket::futures::WebSocket` | 直接 Sink+Stream，与 `async_trait` 最自然配合 | gloo 版本需跟 web-sys 对齐 |
| 手写 `web-sys::WebSocket` + `futures_channel::mpsc` 桥 | 最小依赖，完全控制 | 代码量约 80 行 |

**选 `gloo-net`**：版本稳定，有积极维护，与 wasm-bindgen 0.2 对齐。底层就是 `web-sys::WebSocket`，不引入额外语义风险。

### 2.2 实现

```rust
// helix-driver-wasm/src/transport.rs
use gloo_net::websocket::{futures::WebSocket as GlooWs, Message as GlooMsg};
use futures_channel::mpsc;
use futures_util::{SinkExt, StreamExt};
use wasm_bindgen_futures::spawn_local;
use helix_core::ports::Transport;
use bytes::Bytes;

pub struct WasmTransport {
    /// 入站帧接收端（由 spawn_local read loop 推入）
    inbound_rx: mpsc::UnboundedReceiver<Bytes>,
    /// 出站帧发送端
    outbound_tx: mpsc::UnboundedSender<Bytes>,
    url: String,
}

impl WasmTransport {
    pub fn new(url: String) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::unbounded();
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded::<Bytes>();
        let url_clone = url.clone();

        // 单线程：spawn_local（不是 tokio::spawn）
        spawn_local(async move {
            loop {
                match GlooWs::open(&url_clone) {
                    Ok(ws) => {
                        let (mut sink, mut stream) = ws.split();

                        // 读 loop
                        let inbound_tx_clone = inbound_tx.clone();
                        spawn_local(async move {
                            while let Some(Ok(GlooMsg::Bytes(b))) = stream.next().await {
                                let _ = inbound_tx_clone.unbounded_send(Bytes::from(b));
                            }
                        });

                        // 写 loop
                        while let Some(frame) = outbound_rx.next().await {
                            if sink.send(GlooMsg::Bytes(frame.to_vec())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        web_sys::console::warn_1(&format!("ws connect failed: {e:?}").into());
                    }
                }
                // 退避 1s（wasm 用 gloo-timers）
                gloo_timers::future::TimeoutFuture::new(1_000).await;
            }
        });

        WasmTransport { inbound_rx, outbound_tx, url }
    }
}

// wasm 单线程：?Send
#[async_trait::async_trait(?Send)]
impl Transport for WasmTransport {
    type Error = WasmError;

    async fn send(&mut self, frame: Bytes) -> Result<(), WasmError> {
        self.outbound_tx
            .unbounded_send(frame)
            .map_err(|_| WasmError::ChannelClosed)
    }

    async fn recv(&mut self) -> Result<Bytes, WasmError> {
        self.inbound_rx.next().await.ok_or(WasmError::Closed)
    }

    async fn close(&mut self) -> Result<(), WasmError> {
        self.outbound_tx.close_channel();
        Ok(())
    }
}
```

**关键点**：

- `#[async_trait(?Send)]` 是 wasm32 的专用签名（`JsValue` 是 `!Send`，不加 `?Send` 编译失败）。
- `spawn_local`（来自 `wasm-bindgen-futures`）是 wasm 上唯一合法的"后台任务"。
- 退避用 `gloo_timers::future::TimeoutFuture::new(ms)` 而非 `tokio::time::sleep`。

### 2.3 重连退避

wasm 单线程下退避不阻塞 UI，因为 `TimeoutFuture` 底层是 `setTimeout`，`await` 时让出 JS event loop。指数退避逻辑与 native 侧一致（500ms × 2^attempt，jitter from `js_sys::Math::random()`）：

```rust
async fn wasm_backoff(attempt: u32) {
    let base_ms = 500u32 * 2u32.pow(attempt.min(8));
    let jitter = (js_sys::Math::random() * 500.0) as u32;
    gloo_timers::future::TimeoutFuture::new(base_ms + jitter).await;
}
```

---

## 3. Storage Port 实现：`WasmStorage`

### 3.1 OPFS sahpool VFS 约束

**核心约束**：OPFS synchronous access handle（`createSyncAccessHandle`）**只能在 Web Worker 里调用**，不能在 main thread（会抛 `InvalidStateError`）。

两条路：

| 路径 | 场景 | 说明 |
|---|---|---|
| **Web Worker + OPFS sahpool**（推荐） | 生产 | `sqlite-wasm-rs opfs` feature；Worker 里初始化 sahpool VFS；主线程通过 `postMessage` / `SharedArrayBuffer` 发 SQL 指令 |
| **main thread + async OPFS VFS** | 原型/受限环境 | 较慢（async syscall），但无 Worker 限制；`sqlite-wasm-rs` 亦提供 async VFS |

Helix WASM 驱动选 **"适配器 + Worker bridge"** 模式：driver 侧 `WasmStorage` 暴露 `async fn query/execute`，内部通过 `futures_channel::mpsc` 和 `js_sys::Promise` 把 SQL 发给 Worker，Worker 同步执行后 postMessage 结果回来。

### 3.2 实现骨架

```rust
// helix-driver-wasm/src/storage.rs
use futures_channel::{mpsc, oneshot};
use wasm_bindgen_futures::spawn_local;
use helix_core::ports::{Storage, SqlParam, Row};

/// 一个 SQL 请求，带 oneshot 回报通道
struct SqlRequest {
    sql: String,
    params: Vec<SqlParam>,
    resp_tx: oneshot::Sender<Result<Vec<Row>, WasmError>>,
}

pub struct WasmStorage {
    sql_tx: mpsc::UnboundedSender<SqlRequest>,
}

impl WasmStorage {
    /// 初始化时必须在 Worker context 里调用（OPFS sahpool 约束）
    pub fn new_in_worker() -> Self {
        let (sql_tx, mut sql_rx) = mpsc::unbounded::<SqlRequest>();

        spawn_local(async move {
            // 初始化 OPFS sahpool VFS（Worker 线程内）
            let db = sqlite_wasm_rs::open_with_opfs("helix.db")
                .await
                .expect("OPFS sahpool init");

            while let Some(req) = sql_rx.next().await {
                let result = db.query(&req.sql, &req.params).await
                    .map_err(WasmError::Sqlite);
                let _ = req.resp_tx.send(result);
            }
        });

        WasmStorage { sql_tx }
    }
}

#[async_trait::async_trait(?Send)]
impl Storage for WasmStorage {
    type Error = WasmError;

    async fn execute(&self, sql: &str, params: &[SqlParam]) -> Result<(), WasmError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.sql_tx.unbounded_send(SqlRequest {
            sql: sql.to_string(),
            params: params.to_vec(),
            resp_tx,
        }).map_err(|_| WasmError::ChannelClosed)?;
        resp_rx.await.map_err(|_| WasmError::ChannelClosed)?.map(|_| ())
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> Result<Vec<Row>, WasmError> {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.sql_tx.unbounded_send(SqlRequest {
            sql: sql.to_string(),
            params: params.to_vec(),
            resp_tx,
        }).map_err(|_| WasmError::ChannelClosed)?;
        resp_rx.await.map_err(|_| WasmError::ChannelClosed)?
    }

    async fn query_one(&self, sql: &str, params: &[SqlParam]) -> Result<Option<Row>, WasmError> {
        let rows = self.query(sql, params).await?;
        Ok(rows.into_iter().next())
    }
}
```

**注意**：wasm32 是单线程，`spawn_local` 是协作式调度（不是 OS 线程），上面的 Worker 内 loop 是事件驱动的——`sql_rx.next().await` 让出 JS event loop，JS 收到 SQL 请求 → Worker 同步执行 → postMessage 结果。整体是异步的，但 SQLite 写是同步的（sahpool 提供同步 handle），因此驱动 wasm 时的单写者语义是自然满足的（单 Worker 顺序执行）。

### 3.3 降级：main thread async VFS

若不支持 Worker（如受限 iframe），可用 async VFS 降级：

```rust
/// 降级：main thread 可用，但写入慢（async OPFS）
pub async fn new_async_fallback() -> Self {
    let (sql_tx, mut sql_rx) = mpsc::unbounded::<SqlRequest>();
    spawn_local(async move {
        let db = sqlite_wasm_rs::open_with_opfs_async("helix.db")
            .await
            .expect("async OPFS init");
        while let Some(req) = sql_rx.next().await {
            let result = db.query_async(&req.sql, &req.params).await
                .map_err(WasmError::Sqlite);
            let _ = req.resp_tx.send(result);
        }
    });
    WasmStorage { sql_tx }
}
```

---

## 4. Clock Port 实现：`WasmClock`

```rust
// helix-driver-wasm/src/clock.rs
use helix_core::ports::Clock;

#[derive(Clone)]
pub struct WasmClock;

impl Clock for WasmClock {
    fn now_ms(&self) -> u64 {
        // js_sys::Date::now() 返回 f64 毫秒（Unix epoch），wasm32 唯一安全来源
        js_sys::Date::now() as u64
    }
}
```

**ScheduleTimer effect 的兑现**：

```rust
use gloo_timers::future::TimeoutFuture;
use futures_channel::mpsc;
use std::collections::HashMap;
use std::rc::Rc;
use std::cell::RefCell;

/// wasm 单线程：用 Rc<RefCell<>> 而非 Arc<Mutex<>>
pub struct WasmTimerRegistry {
    /// timer_id → AbortHandle（gloo-timers 的取消句柄）
    handles: Rc<RefCell<HashMap<u64, gloo_timers::future::TimeoutHandle>>>,
}

impl WasmTimerRegistry {
    pub fn schedule(
        &self,
        timer_id: u64,
        after_ms: u64,
        engine_tx: mpsc::UnboundedSender<helix_core::engine::Tick>,
    ) {
        let engine_tx_clone = engine_tx.clone();
        let handles = self.handles.clone();

        // gloo_timers::future::TimeoutFuture 是 JavaScript setTimeout 的 Future 封装
        let handle = gloo_timers::callback::Timeout::new(
            after_ms as u32,
            move || {
                let _ = engine_tx_clone.unbounded_send(
                    helix_core::engine::Tick::Timer(timer_id)
                );
                handles.borrow_mut().remove(&timer_id);
            }
        );
        self.handles.borrow_mut().insert(timer_id, handle);
    }

    pub fn cancel(&self, timer_id: u64) {
        // drop handle = 取消 setTimeout
        self.handles.borrow_mut().remove(&timer_id);
    }
}
```

**与 native 不同的地方**：

- `Rc<RefCell<>>` 而非 `Arc<Mutex<>>`：wasm32 是单线程，`Rc/RefCell` 足够且比 `Arc/Mutex` 轻。
- `gloo_timers::callback::Timeout` 底层是 `clearTimeout`（drop 时自动取消）。
- 不需要 `tokio::spawn` + `abort()` 那套，JS setTimeout 天然支持取消。

---

## 5. EventSink Port 实现：`WasmEventSink`

wasm 侧没有 tokio broadcast，用 `futures_channel::mpsc::unbounded`（单消费者）或直接在 `spawn_local` 里回调 JS：

```rust
// helix-driver-wasm/src/event_sink.rs
use helix_core::ports::EventSink;
use helix_core::events::DomainEvent;
use wasm_bindgen::prelude::*;

/// wasm 侧 EventSink：把 DomainEvent 序列化后调 JS 回调
#[derive(Clone)]
pub struct WasmEventSink {
    callback: js_sys::Function,
}

impl WasmEventSink {
    /// `callback` 是 JS 注册的 onEvent(channel: string, payload: JsValue) 函数
    pub fn new(callback: js_sys::Function) -> Self {
        WasmEventSink { callback }
    }
}

impl EventSink for WasmEventSink {
    fn emit(&self, event: DomainEvent) {
        if let Some((channel, payload)) = domain_event_to_wire(&event) {
            let js_payload = serde_wasm_bindgen::to_value(&payload)
                .unwrap_or(JsValue::NULL);
            let _ = self.callback.call2(
                &JsValue::NULL,
                &JsValue::from_str(channel),
                &js_payload,
            );
        }
    }
}

/// wire 名映射（与 native 侧 NativeEventSink 镜像一致）
fn domain_event_to_wire(event: &DomainEvent) -> Option<(&'static str, serde_json::Value)> {
    // 与 helix-driver-native 的 domain_event_to_wire 函数完全相同
    // 两侧共享同一映射表（可提取到 helix-core 的 event 模块作为纯函数）
    use DomainEvent::*;
    match event {
        PostReceived { post, .. } =>
            Some(("im:post:received", serde_json::to_value(post).ok()?)),
        PostUpdated { post, .. } =>
            Some(("im:post:updated", serde_json::to_value(post).ok()?)),
        // ...
        _ => None,
    }
}
```

**wasm EventSink 设计要点**：

- JS 侧通过 `wasm-bindgen` 传入一个回调函数，wasm 调它发送事件。
- 也可以换成 `CustomEvent` dispatch（`web_sys::CustomEvent::new` + `window.dispatch_event`），让 Angular 通过 DOM 事件监听——但直接 callback 更轻量。
- `im:__bus__` 逻辑在 JS 宿主侧处理（与 Tauri 宿主的 bus loop 对等）。

---

## 6. HttpRequester Port 实现：`WasmHttpRequester`

```rust
// helix-driver-wasm/src/http_requester.rs
use helix_core::ports::HttpRequester;
use helix_core::effect::{HttpRequest, HttpResponse, HttpMethod};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{RequestInit, RequestMode, Headers};

#[derive(Clone)]
pub struct WasmHttpRequester {
    base_url: String,
    // 运行时 headers（token / connectionId）
    headers: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, String>>>,
}

impl WasmHttpRequester {
    pub fn new(base_url: String) -> Self {
        WasmHttpRequester {
            base_url,
            headers: std::rc::Rc::new(std::cell::RefCell::new(Default::default())),
        }
    }

    pub fn set_header(&self, key: &str, value: &str) {
        self.headers.borrow_mut().insert(key.to_string(), value.to_string());
    }
}

#[async_trait::async_trait(?Send)]
impl HttpRequester for WasmHttpRequester {
    type Error = WasmError;

    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, WasmError> {
        let url = format!("{}{}", self.base_url, req.path);
        let window = web_sys::window().ok_or(WasmError::NoWindow)?;

        let mut init = RequestInit::new();
        init.method(match req.method {
            HttpMethod::Get  => "GET",
            HttpMethod::Post => "POST",
        });
        init.mode(RequestMode::Cors);

        let js_headers = Headers::new().map_err(|_| WasmError::Fetch("headers".into()))?;
        for (k, v) in self.headers.borrow().iter() {
            js_headers.set(k, v).ok();
        }
        if let Some(body) = &req.body {
            let body_str = std::str::from_utf8(body)
                .map_err(|_| WasmError::Fetch("utf8".into()))?;
            init.body(Some(&wasm_bindgen::JsValue::from_str(body_str)));
            js_headers.set("Content-Type", "application/json").ok();
        }
        init.headers(js_headers.as_ref());

        let request = web_sys::Request::new_with_str_and_init(&url, &init)
            .map_err(|_| WasmError::Fetch("request init".into()))?;

        let resp_value = JsFuture::from(window.fetch_with_request(&request)).await
            .map_err(|e| WasmError::Fetch(format!("{e:?}")))?;

        let resp: web_sys::Response = resp_value.dyn_into()
            .map_err(|_| WasmError::Fetch("dyn_into Response".into()))?;
        let status = resp.status();

        let body_array_buffer = JsFuture::from(
            resp.array_buffer().map_err(|_| WasmError::Fetch("array_buffer".into()))?
        ).await.map_err(|e| WasmError::Fetch(format!("{e:?}")))?;

        let body_uint8 = js_sys::Uint8Array::new(&body_array_buffer);
        let body_bytes = body_uint8.to_vec();

        Ok(HttpResponse {
            status,
            body: bytes::Bytes::from(body_bytes),
        })
    }
}
```

---

## 7. wasm 侧 CoreEngine 驱动循环

wasm 单线程，engine loop 用 `spawn_local` + `futures_channel` 串行化：

```rust
// helix-driver-wasm/src/runtime.rs
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use futures_channel::mpsc;
use std::rc::Rc;
use std::cell::RefCell;

/// wasm 宿主入口（被 JS 调用）
#[wasm_bindgen]
pub struct HelixWasm {
    /// engine 的 Tick 入口
    engine_tx: mpsc::UnboundedSender<helix_core::engine::Tick>,
}

#[wasm_bindgen]
impl HelixWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(ws_url: &str, http_base: &str, on_event: js_sys::Function) -> HelixWasm {
        let transport  = WasmTransport::new(ws_url.to_string());
        let storage    = WasmStorage::new_async_fallback();  // 或 new_in_worker()
        let clock      = WasmClock;
        let event_sink = WasmEventSink::new(on_event);
        let http       = WasmHttpRequester::new(http_base.to_string());

        let (engine_tx, engine_rx) = mpsc::unbounded::<helix_core::engine::Tick>();
        let engine_tx_clone = engine_tx.clone();

        // 注意：wasm32 单线程，Rc<RefCell<>> 替代 Arc<Mutex<>>
        let engine = Rc::new(RefCell::new(
            helix_core::CoreEngine::new(/* ports */)
        ));

        spawn_local(engine_loop(engine_rx, engine, storage, transport, http, event_sink));

        HelixWasm { engine_tx }
    }

    /// JS 侧用户意图入口
    #[wasm_bindgen]
    pub fn send_command(&self, cmd_json: &str) {
        if let Ok(cmd) = serde_json::from_str(cmd_json) {
            let _ = self.engine_tx.unbounded_send(
                helix_core::engine::Tick::AppCommand(Box::new(cmd))
            );
        }
    }
}

async fn engine_loop(
    mut engine_rx: mpsc::UnboundedReceiver<helix_core::engine::Tick>,
    engine: Rc<RefCell<helix_core::CoreEngine>>,
    storage: WasmStorage,
    mut transport: WasmTransport,
    http: WasmHttpRequester,
    event_sink: WasmEventSink,
) {
    while let Some(tick) = engine_rx.next().await {
        let effects = engine.borrow_mut().step(tick)
            .unwrap_or_default();
        execute_effects_wasm(effects, &storage, &mut transport, &http, &event_sink).await;
    }
}
```

**单线程并发语义**：

- 所有 `spawn_local` 任务（WS recv loop / engine loop / storage loop）在同一个 JS event loop 上协作调度。
- 不需要 `Mutex`：`Rc<RefCell<>>` 足够（单线程不存在数据竞争）。
- `engine.borrow_mut().step(tick)` 同步执行（纯 CPU），不会产生 await 点，不会被其他任务抢占——单线程确定性天然满足。

---

## 8. wasm-bindgen 对外 API 设计原则

1. **只暴露 `String` / 基本类型 / `JsValue`**，不暴露 Rust 内部类型（如 `DomainEvent`）。
2. **命令入口用 JSON 字符串**（`send_command(cmd_json: &str)`），避免复杂 wasm-bindgen 类型绑定。
3. **事件出口用 JS callback**（`on_event: js_sys::Function`），JS 侧自己解析 `(channel, payload)`。
4. `#[wasm_bindgen]` 标注的公开 struct 必须 `Clone` 或在文档中说明所有权。

---

## 9. 体积控制（wasm binary size）

Cargo.toml profile（已在三栖约束底座文档声明，此处重申关键点）：

```toml
[profile.release]
opt-level     = "z"   # 最小体积
lto           = true
codegen-units = 1
panic         = "abort"  # 去掉 unwind 表，对 wasm 额外省 10-20%
strip         = true
```

**构建流程**：

```bash
# 1. 编译（wasm-pack 推荐，会自动调 wasm-opt）
wasm-pack build crates/helix-driver-wasm --target web --release

# 2. 手动 wasm-opt（若不用 wasm-pack）
wasm-opt -Oz --enable-mutable-globals \
    target/wasm32-unknown-unknown/release/helix_driver_wasm.wasm \
    -o dist/helix.wasm

# 3. 体积分析
twiggy top -n 20 target/wasm32-unknown-unknown/release/helix_driver_wasm.wasm
```

**特别注意**：`helix-im` 的 `serde_json` 反序列化路径是 wasm binary 里体积大户之一。用 `twiggy dominators` 找出来后，考虑：

- 把大型 JSON 结构改为逐字段解析（`serde_json::from_str` → 手工 `Value` 解引）。
- 或用 `simd_json`（有 wasm 支持，但体积换速度需权衡）。

---

## 10. Port 实现汇总

| core Port | Wasm 实现类型 | 核心依赖 | 单线程并发工具 |
|---|---|---|---|
| `Transport` | `WasmTransport` | `gloo-net::websocket` | `spawn_local` + `mpsc::unbounded` |
| `Storage` | `WasmStorage` | `sqlite-wasm-rs` + OPFS | `spawn_local` + `oneshot` 回报 |
| `Clock` | `WasmClock` | `js_sys::Date::now()` | 无状态 Clone |
| `EventSink` | `WasmEventSink` | `js_sys::Function` 回调 | 直接调用 (同步) |
| `HttpRequester` | `WasmHttpRequester` | `web-sys fetch` + `JsFuture` | `spawn_local` 内 await |
| `IdGen` | `WasmIdGen` | `uuid::Uuid::new_v4()` + getrandom[js] | 无状态 Clone |
| Timer | `WasmTimerRegistry` | `gloo_timers::Timeout` | `Rc<RefCell<HashMap>>` |

---

## 11. 踩坑备档

1. **OPFS sahpool 只能在 Worker**：在 main thread 调 `createSyncAccessHandle` 抛 `InvalidStateError`。生产环境必须 Worker 桥接；原型/测试可用 async VFS 降级。
2. **`?Send` 是必须的**：`web-sys` 所有类型都是 `!Send`（JS 对象不能跨线程）。`#[async_trait(?Send)]` 是 wasm Transport/Storage 实现的正确签名，忘了加会在 impl 时报 "the trait `Send` is not implemented for `JsValue`"。
3. **`Rc<RefCell<>>` 替代 `Arc<Mutex<>>`**：wasm 单线程无需 Arc，RefCell 运行时借用检查比 Mutex 死锁风险更低、性能更好。
4. **`js_sys::Date::now()` 精度**：浏览器因安全限制（Spectre）会舍入到 ~1ms。对 IM 的 `now_ms` 打时间戳够用，但不适合高精度基准测试。
5. **getrandom workspace 级**：必须在 workspace `[dependencies]` 节声明 `getrandom = { version = "0.2", features = ["js"] }`，不只是在 `helix-driver-wasm` 里声明——否则经 helix-core 传递的依赖路径仍会 link 失败。
