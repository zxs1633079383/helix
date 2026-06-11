# helix-driver-native —— Native 驱动层设计文档

> **版本** 2026-06-08 · 基于 cses-client 源码实测 + 三栖约束底座
>
> **本文目的**：详述 `helix-driver-native` 如何用 tokio/sqlx/tokio-tungstenite/reqwest **实现全部 core ports**，并说明多线程并发模型（WS runtime / 单写者 / 4-worker 分片）如何驻扎在驱动层而非 core。

---

## 0. 定位与依赖方向

```
┌─────────────────────────────────────────────────────────┐
│  helix-tauri (宿主, Tauri State 注入)                   │
│  helix-driver-native (本文档)                           │
│    implements: Transport / Storage / Clock / EventSink  │
│                HttpRequester / IdGen / Clock            │
│    owns:       tokio runtime / sqlx pool / reqwest      │
│                WS 重连状态机 / 单写者单例                │
├─────────────────────────────────────────────────────────┤
│  helix-core  (port traits)                              │
│  helix-im    (deterministic business module)            │
└─────────────────────────────────────────────────────────┘
```

依赖单向：`helix-driver-native` → `helix-core`，反向依赖禁止。

---

## 1. Cargo.toml

```toml
# crates/helix-driver-native/Cargo.toml
[package]
name    = "helix-driver-native"
version = "0.1.0"
edition = "2021"

[dependencies]
helix-core = { path = "../helix-core" }
bytes       = { workspace = true }
thiserror   = { workspace = true }
serde       = { workspace = true }
serde_json  = "1"
tracing     = { workspace = true }

# ---- 仅 non-wasm target（驱动三件套）----
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1", features = [
    "rt-multi-thread", "macros", "sync",
    "time", "net", "io-util"
]}
sqlx = { version = "0.8", default-features = false, features = [
    "sqlite", "runtime-tokio",
    "macros", "migrate"
]}
tokio-tungstenite = { version = "0.23", features = ["native-tls"] }
reqwest = { version = "0.12", default-features = false, features = [
    "json", "rustls-tls", "stream"
]}
dashmap  = "6"
rand     = "0.8"
uuid     = { version = "1", features = ["v4"] }
```

**为何全在 `cfg(not(wasm32))` 块**：驱动层 crate 允许被 workspace CI 用 native target 编译，但 Cargo 必须看不到 wasm-only crate（`wasm-bindgen` 等）和 native-only crate（`mio`/`tokio-net`）同时出现。`target.'cfg(...)'` 是 Cargo 唯一正确的隔离手段。

---

## 2. Transport Port 实现：`NativeTransport`

### 2.1 职责边界

`NativeTransport` 对 core 只暴露三个语义：**发一帧 / 收一帧 / 连接状态变更（driver 主动喂 lifecycle event）**。WS 握手、重连退避、心跳 ping/pong——全是驱动私有细节，不泄漏给 core。

### 2.2 核心结构

```rust
// helix-driver-native/src/transport.rs

/// WS 连接状态机（driver 私有，core 不可见）
#[derive(Debug, Clone, PartialEq)]
enum WsState {
    Disconnected,
    Connecting,
    Connected,
    Reconnecting { attempt: u32, next_try_at: tokio::time::Instant },
}

/// driver → core engine 的 outbox：incoming WS 帧 + lifecycle 事件
/// 使用 futures_channel::mpsc（不是 tokio channel，core-compatible）
pub struct NativeTransport {
    /// 往 WS 写的 sender；None 表示未连接
    tx: Arc<Mutex<Option<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>>,
    /// driver 内部 tokio spawn 的 recv loop 把收到的帧推进来
    inbound_rx: futures_channel::mpsc::Receiver<Bytes>,
    state: Arc<Mutex<WsState>>,
    url: String,
    /// 供驱动层触发 reconnect 的 handle
    reconnect_tx: tokio::sync::watch::Sender<()>,
}
```

### 2.3 `#[async_trait] Transport` 实现

```rust
#[async_trait::async_trait]
impl helix_core::ports::Transport for NativeTransport {
    type Error = NativeError;

    async fn send(&mut self, frame: Bytes) -> Result<(), NativeError> {
        let mut guard = self.tx.lock().await;
        let sink = guard.as_mut().ok_or(NativeError::NotConnected)?;
        sink.send(Message::Binary(frame.to_vec())).await
            .map_err(NativeError::Ws)
    }

    async fn recv(&mut self) -> Result<Bytes, NativeError> {
        self.inbound_rx.next().await.ok_or(NativeError::Closed)
    }

    async fn close(&mut self) -> Result<(), NativeError> {
        let mut guard = self.tx.lock().await;
        if let Some(sink) = guard.as_mut() {
            let _ = sink.close().await;
        }
        *guard = None;
        Ok(())
    }
}
```

### 2.4 WS Runtime（`tokio::spawn` 驱动层私有 loop）

重连、心跳、split 管理——全驻在 `WsRuntime` 内，core engine 完全不知道这一层：

```rust
/// 驱动层私有，不进 helix-core
pub(crate) struct WsRuntime {
    url: String,
    inbound_tx: futures_channel::mpsc::Sender<Bytes>,
    state: Arc<Mutex<WsState>>,
    reconnect_rx: tokio::sync::watch::Receiver<()>,
}

impl WsRuntime {
    pub fn start(mut self) {
        // 这里 tokio::spawn 只在 native driver 里用
        tokio::spawn(async move {
            loop {
                match self.connect_once().await {
                    Ok(_) => {
                        // 连接正常关闭 → 立即重连
                        tracing::info!("ws: closed cleanly, reconnecting");
                    }
                    Err(e) => {
                        tracing::warn!("ws: error {e}, backing off");
                    }
                }
                self.backoff().await;
            }
        });
    }

    async fn connect_once(&mut self) -> Result<(), NativeError> {
        let (ws, _) = tokio_tungstenite::connect_async(&self.url).await
            .map_err(NativeError::Ws)?;
        let (mut sink, mut stream) = ws.split();

        {
            let mut state = self.state.lock().await;
            *state = WsState::Connected;
        }

        // 心跳 loop + recv loop 并发
        let inbound_tx = self.inbound_tx.clone();
        let read_task = tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                match msg {
                    Message::Binary(b) => {
                        let _ = inbound_tx.unbounded_send(Bytes::from(b));
                    }
                    Message::Pong(_) => {}  // readDeadline 重置由 ping task 处理
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        });

        // ping 每 25s 一次
        let ping_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(
                std::time::Duration::from_secs(25)
            );
            loop {
                interval.tick().await;
                if sink.send(Message::Ping(vec![])).await.is_err() {
                    break;
                }
            }
        });

        tokio::select! {
            _ = read_task => {}
            _ = ping_task => {}
        }
        Ok(())
    }

    /// 指数退避 + jitter，防重连风暴
    async fn backoff(&self) {
        let attempt = {
            let state = self.state.lock().await;
            if let WsState::Reconnecting { attempt, .. } = *state { attempt } else { 0 }
        };
        let base_ms = 500u64 * 2u64.pow(attempt.min(8));
        let jitter = rand::random::<u64>() % 500;
        tokio::time::sleep(
            std::time::Duration::from_millis(base_ms + jitter)
        ).await;
    }
}
```

**关键设计决策**：

- `tokio::spawn` 只在 `WsRuntime::start()` 里调用，不进 `helix-core`。wasm 下 `WsRuntime` 不存在，由 `helix-driver-wasm` 的 `spawn_local` 替代。
- `inbound_rx` 是 `futures_channel::mpsc::Receiver`（不是 `tokio::sync::mpsc`），保证如果 `Transport` trait 实现被挪到 core 边界测试也能编译。
- hello 握手（`connectionId` 回注 header）：driver 收到 `type=hello` 帧 → 通过 core EventSink 吐 `DomainEvent::Connected { connection_id }` → helix-tauri 宿主监听并调用 `HeaderManager` 更新 `Arc<RwLock<HeaderMap>>`——这条链路完全在 core 之外。

---

## 3. Storage Port 实现：`NativeStorage`

### 3.1 读写分离架构（单写者防 SQLITE_BUSY）

```
                 ┌──────────────────────────────────┐
                 │  NativeStorage (Arc 共享)         │
                 │                                   │
  Storage::      │  write_pool: SqlitePool(1 conn)  │  ← 单写者
  execute()  ───►│  read_pool:  SqlitePool(4 conn)  │  ← WAL 多读
  query()    ───►│  db_path: PathBuf                 │
                 └──────────────────────────────────┘
```

```rust
// helix-driver-native/src/storage.rs
use sqlx::{SqlitePool, sqlite::{SqliteConnectOptions, SqliteJournalMode, SqliteSynchronous}};
use std::path::PathBuf;
use std::sync::Arc;

pub struct NativeStorage {
    /// 单写者连接池（max_connections=1）
    write_pool: SqlitePool,
    /// WAL 读连接池（多并发读）
    read_pool: SqlitePool,
}

impl NativeStorage {
    pub async fn open(db_path: &PathBuf) -> Result<Self, NativeError> {
        let opts_write = SqliteConnectOptions::new()
            .filename(db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(std::time::Duration::from_secs(5))
            .max_connections(1);                              // 单写者铁律

        let opts_read = SqliteConnectOptions::new()
            .filename(db_path)
            .read_only(true)                                  // 读连接只读
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(3))
            .max_connections(4);                              // WAL 并发读

        Ok(NativeStorage {
            write_pool: SqlitePool::connect_with(opts_write).await
                .map_err(NativeError::Sqlx)?,
            read_pool: SqlitePool::connect_with(opts_read).await
                .map_err(NativeError::Sqlx)?,
        })
    }

    /// 运行 migrations（必须在 start 时调用）
    pub async fn migrate(&self) -> Result<(), NativeError> {
        sqlx::migrate!("./migrations")
            .run(&self.write_pool).await
            .map_err(NativeError::Migrate)
    }
}
```

### 3.2 `Storage` port 实现

core port 的 `execute` 走写池，`query` 走读池——WAL 模式下互不阻塞：

```rust
#[async_trait::async_trait]
impl helix_core::ports::Storage for NativeStorage {
    type Error = NativeError;

    async fn execute(&self, sql: &str, params: &[SqlParam]) -> Result<(), NativeError> {
        let mut q = sqlx::query(sql);
        for p in params {
            q = bind_param(q, p);
        }
        q.execute(&self.write_pool).await
            .map(|_| ())
            .map_err(NativeError::Sqlx)
    }

    async fn query(&self, sql: &str, params: &[SqlParam]) -> Result<Vec<Row>, NativeError> {
        let mut q = sqlx::query(sql);
        for p in params {
            q = bind_param(q, p);
        }
        q.fetch_all(&self.read_pool).await
            .map(|rows| rows.into_iter().map(Row::from).collect())
            .map_err(NativeError::Sqlx)
    }

    async fn query_one(&self, sql: &str, params: &[SqlParam]) -> Result<Option<Row>, NativeError> {
        let mut q = sqlx::query(sql);
        for p in params {
            q = bind_param(q, p);
        }
        q.fetch_optional(&self.read_pool).await
            .map(|opt| opt.map(Row::from))
            .map_err(NativeError::Sqlx)
    }
}
```

### 3.3 `batch_upsert` 热路径（O(1) 每行，核心是 SQL 参数化）

IM 的 `batch_upsert(table, rows, conflict_key)` 最终通过 core Storage port 表达。**conflict_key（如 `temporary_id`）是参数，不进 driver 代码**：

```rust
/// helix-im 调 core Storage port 时拼出的 SQL 骨架（这段代码在 helix-im，不在 driver）
// INSERT INTO messages(...) VALUES(?) ON CONFLICT(temporary_id)
// DO UPDATE SET id=excluded.id, content=excluded.content, ...
// 热路径：每行一次 O(log N) INSERT，无 read-modify-write
```

driver 只负责"把 SQL + params 忠实发给 SQLite"，不知道 IM 的表名或列集。

### 3.4 多租户切库（driver 层）

```rust
/// DbManager 按 tenant_key 管理多个 NativeStorage 实例
pub struct DbManager {
    /// DashMap：tenant_key → Arc<NativeStorage>
    stores: dashmap::DashMap<String, Arc<NativeStorage>>,
    app_data_dir: PathBuf,
}

impl DbManager {
    /// 惰性打开：首次访问时创建 db.sqlite
    pub async fn get_or_open(&self, tenant_key: &str) -> Result<Arc<NativeStorage>, NativeError> {
        if let Some(s) = self.stores.get(tenant_key) {
            return Ok(s.clone());
        }
        let db_path = self.app_data_dir
            .join("tenants")
            .join(tenant_key)
            .join("db.sqlite");
        tokio::fs::create_dir_all(db_path.parent().unwrap()).await?;
        let storage = Arc::new(NativeStorage::open(&db_path).await?);
        storage.migrate().await?;
        self.stores.insert(tenant_key.to_string(), storage.clone());
        Ok(storage)
    }
}
```

路径格式：`${app_data_dir}/tenants/${tenant_key}/db.sqlite`——与 cses-client 实测一致。

---

## 4. Clock Port 实现：`NativeClock`

```rust
// helix-driver-native/src/clock.rs
#[derive(Clone)]
pub struct NativeClock;

impl helix_core::ports::Clock for NativeClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .expect("system clock before epoch")
            .as_millis() as u64
    }
}
```

**ScheduleTimer effect 的兑现**（driver 私有，不进 core）：

```rust
/// driver 的 effect 执行器（execute_effects 函数中的 Timer 分支）
async fn handle_schedule_timer(
    timer_id: u64,
    after_ms: u64,
    engine: Arc<Mutex<CoreEngine>>,
    effect_tx: futures_channel::mpsc::Sender<Vec<Effect>>,
) {
    tokio::time::sleep(std::time::Duration::from_millis(after_ms)).await;
    // sleep 结束 → 把 TimerFired 喂回 engine.step()
    let mut engine = engine.lock().await;
    let effects = engine.step(Tick::Timer(timer_id))
        .unwrap_or_default();
    let _ = effect_tx.unbounded_send(effects);
}

/// CancelTimer：driver 维护一个 tokio JoinHandle 表，cancel 时 abort
struct TimerRegistry {
    handles: dashmap::DashMap<u64, tokio::task::JoinHandle<()>>,
}

impl TimerRegistry {
    fn schedule(&self, timer_id: u64, after_ms: u64, engine: Arc<Mutex<CoreEngine>>,
                 effect_tx: futures_channel::mpsc::Sender<Vec<Effect>>) {
        let handle = tokio::spawn(
            handle_schedule_timer(timer_id, after_ms, engine, effect_tx)
        );
        self.handles.insert(timer_id, handle);
    }

    fn cancel(&self, timer_id: u64) {
        if let Some((_, handle)) = self.handles.remove(&timer_id) {
            handle.abort();
        }
    }
}
```

这里 `tokio::spawn` 和 `tokio::time::sleep` 只在 `helix-driver-native` 里出现。

---

## 5. EventSink Port 实现：`NativeEventSink`

EventSink 是驱动层把 `DomainEvent` 序列化后通知宿主的出口。**`im:__bus__` 总线、envelope 格式、wire 名——全在这里，不在 core**：

```rust
// helix-driver-native/src/event_sink.rs
use helix_core::ports::EventSink;
use helix_core::events::DomainEvent;

/// native 侧 EventSink：用 tokio broadcast 做 fan-out
#[derive(Clone)]
pub struct NativeEventSink {
    tx: tokio::sync::broadcast::Sender<DomainEvent>,
}

impl NativeEventSink {
    pub fn new() -> (Self, tokio::sync::broadcast::Receiver<DomainEvent>) {
        let (tx, rx) = tokio::sync::broadcast::channel(1024);
        (NativeEventSink { tx }, rx)
    }
}

impl EventSink for NativeEventSink {
    fn emit(&self, event: DomainEvent) {
        // broadcast：lagged 接收方自动丢弃过老消息
        let _ = self.tx.send(event);
    }
}

/// 用在 helix-tauri 宿主侧：监听 broadcast → 序列化 → tauri emit
/// wire 名（"im:post:received" 等）只在此处出现
pub fn domain_event_to_wire(event: &DomainEvent) -> Option<(&'static str, serde_json::Value)> {
    use DomainEvent::*;
    match event {
        PostReceived { post, .. } => {
            Some(("im:post:received", serde_json::to_value(post).ok()?))
        }
        PostUpdated { post, .. } => {
            Some(("im:post:updated", serde_json::to_value(post).ok()?))
        }
        PostDeleted { post_id, .. } => {
            Some(("im:post:deleted", serde_json::json!({ "postId": post_id })))
        }
        // ... 其他 DomainEvent 变体
        _ => None,
    }
}
```

Tauri 宿主里的 bus loop（见 `docs/04-tauri-host.md`）消费此 broadcast，套 envelope 通过 `im:__bus__` 单通道发往前端。

---

## 6. HttpRequester Port 实现：`NativeHttpRequester`

```rust
// helix-driver-native/src/http_requester.rs
use helix_core::ports::HttpRequester;
use helix_core::effect::{HttpRequest, HttpResponse};
use reqwest::{Client, header::{HeaderMap, HeaderValue}};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 可运行时注入 Header（connectionId / token）
#[derive(Clone)]
pub struct NativeHttpRequester {
    client: Client,
    /// 运行时可更新的公共 headers（如 X-Connection-Id / Authorization）
    default_headers: Arc<RwLock<HeaderMap>>,
    base_url: String,
}

impl NativeHttpRequester {
    pub fn new(base_url: String) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .pool_max_idle_per_host(5)
            .build()
            .expect("reqwest client init");
        NativeHttpRequester {
            client,
            default_headers: Arc::new(RwLock::new(HeaderMap::new())),
            base_url,
        }
    }

    pub async fn set_header(&self, key: &'static str, value: &str) {
        let mut headers = self.default_headers.write().await;
        headers.insert(key, HeaderValue::from_str(value).unwrap());
    }
}

#[async_trait::async_trait]
impl HttpRequester for NativeHttpRequester {
    type Error = NativeError;

    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, NativeError> {
        let url = format!("{}{}", self.base_url, req.path);
        let default_headers = self.default_headers.read().await;
        let mut builder = match req.method {
            helix_core::effect::HttpMethod::Get  => self.client.get(&url),
            helix_core::effect::HttpMethod::Post => self.client.post(&url),
        };
        // 注入公共 headers
        for (k, v) in default_headers.iter() {
            builder = builder.header(k, v);
        }
        // 模块填的请求体（URL、path、body 全是 helix-im 传进来的参数）
        if let Some(body) = req.body {
            builder = builder.body(body);
        }
        let resp = builder.send().await.map_err(NativeError::Http)?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(NativeError::Http)?;
        Ok(HttpResponse { status, body })
    }
}
```

**`/posts/create` 与 `/sync/notify` URL 是 helix-im 拼的**，进入 `HttpRequest.path`，NativeHttpRequester 完全不知道这两个端点。

---

## 7. 并发模型：WS loop / 单写者 / effect 执行器

### 7.1 三个 tokio 任务（driver 层私有）

```
┌──────────────┐    inbound_rx (futures_channel)    ┌──────────────────────┐
│ WsRuntime    │  ─────────────────────────────────► │                      │
│ (recv loop)  │                                     │  CoreEngine loop     │
│              │◄────────────────────────────────── │  (engine.step())     │
└──────────────┘    outbound_tx (发 WS 帧 effect)    │                      │
                                                     │  单线程顺序 step     │
┌──────────────┐    http_response (futures_channel)  │  (Arc<Mutex<>>)     │
│ HttpRuntime  │  ─────────────────────────────────► │                      │
│ (reqwest)    │                                     └──────────────────────┘
└──────────────┘                                              │
                                                              │ effects Vec
                                                              ▼
┌──────────────────────────────────────────────────────────────────────────────┐
│ EffectExecutor（tokio spawn 的 driver 私有任务）                             │
│  Persist → sqlx write_pool（单写者）                                        │
│  Send    → WsRuntime tx                                                     │
│  Http    → NativeHttpRequester                                              │
│  Timer   → TimerRegistry + tokio::time::sleep                              │
│  Emit    → NativeEventSink broadcast                                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

**CoreEngine loop 是单线程顺序执行**（`Arc<Mutex<CoreEngine>>`），保证 engine.step() 不并发——这是 sans-IO 确定性的实现保证。多个 tokio 任务（WS recv / HTTP / Timer 到期）并发产出 `Tick`，但它们都通过同一个 `engine_tx: futures_channel::mpsc::Sender<Tick>` 串行化，engine loop 依次 `while let Some(tick) = engine_rx.next().await { step(tick) }`。

### 7.2 effect 执行器伪代码

```rust
async fn effect_executor_loop(
    mut effects_rx: futures_channel::mpsc::Receiver<Vec<Effect>>,
    storage: Arc<NativeStorage>,
    transport: Arc<Mutex<NativeTransport>>,
    http: NativeHttpRequester,
    event_sink: NativeEventSink,
    timer_registry: Arc<TimerRegistry>,
    engine_tx: futures_channel::mpsc::Sender<Tick>,
) {
    while let Some(effects) = effects_rx.next().await {
        for effect in effects {
            match effect {
                Effect::Persist { sql, params, correlation_id } => {
                    let ok = storage.execute(&sql, &params).await.is_ok();
                    // 写结果作为 Tick 喂回 engine
                    let _ = engine_tx.unbounded_send(
                        Tick::StorageReply { correlation_id, ok }
                    );
                }
                Effect::Send(frame) => {
                    let mut t = transport.lock().await;
                    let _ = t.send(frame).await;
                }
                Effect::HttpRequest { req, correlation_id } => {
                    let http = http.clone();
                    let engine_tx = engine_tx.clone();
                    // HTTP 请求并发跑，响应喂回 engine
                    tokio::spawn(async move {
                        let result = http.request(req).await;
                        let _ = engine_tx.unbounded_send(
                            Tick::HttpReply { correlation_id, result: result.map_err(|e| e.to_string()) }
                        );
                    });
                }
                Effect::ScheduleTimer { id, after_ms } => {
                    timer_registry.schedule(id, after_ms, /* engine ref */, /* effects_tx ref */);
                }
                Effect::CancelTimer { id } => {
                    timer_registry.cancel(id);
                }
                Effect::Emit(event) => {
                    event_sink.emit(event);
                }
            }
        }
    }
}
```

---

## 8. Port 实现汇总表

| core Port | Native 实现类型 | 核心依赖 | 并发角色 |
|---|---|---|---|
| `Transport` | `NativeTransport` | `tokio-tungstenite` | 读：WsRuntime spawn；写：driver 调 |
| `Storage` | `NativeStorage` | `sqlx` | 写：单写池(1 conn)；读：多读池(4 conn) |
| `Clock` | `NativeClock` | `std::time::SystemTime` | 无状态 Clone |
| `EventSink` | `NativeEventSink` | `tokio::sync::broadcast` | fan-out 多消费者 |
| `HttpRequester` | `NativeHttpRequester` | `reqwest` | 连接池 + Arc 共享 |
| `IdGen` | `NativeIdGen` | `uuid::Uuid::new_v4()` | 无状态 Clone |

---

## 9. 踩坑备档

1. **单写者铁律**：`SqlitePool(max_connections=1)` 防 `SQLITE_BUSY`。读池用 `read_only(true)` + WAL，读不阻写。
2. **重连退避 + jitter**：`WsRuntime::backoff()` 上限 500×2^8=128s，jitter 防多实例同时重连。
3. **futures_channel vs tokio::sync**：`engine.step()` 入口的 `Tick` 通道用 `futures_channel::mpsc`，保证 core crate 只依赖 futures-channel，不依赖 tokio（测试时可不起 tokio runtime）。
4. **HttpRequest 并发但 Tick 串行**：HTTP 请求可以多个并发在 tokio 里飞，但响应通过 `engine_tx.unbounded_send` 串行喂回 CoreEngine，engine.step() 单线程处理，保证确定性。
5. **Storage PersistResult 回报**：effect 携带 `correlation_id`，执行完写入结果 `Tick::StorageReply { correlation_id, ok }` 喂回 engine——这是 peek/commit 写成功才推 cursor 的核心机制（§6 文档约束的 driver 侧实现）。
