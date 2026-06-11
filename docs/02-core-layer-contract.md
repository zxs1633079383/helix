# Helix Core Layer — 完整契约设计

> 版本 2026-06-08  
> 事实来源：`01-im-as-core-contract-demands.md`（IM 第一性需求）+ 三栖约束底座 + FRESH_IM_SOURCE 源码实测  
> 本文是 `helix-core` crate 的权威设计文档。所有 trait 签名草案、模块契约、effect 模型在此定稿。

---

## 0. 设计北极星（读这一节就够定方向）

```
三栖命题：同一份业务模块代码在 native(tokio) / wasm(browser) / ffi(C ABI) 三端行为完全一致。
解法：hexagonal ports & adapters + sans-IO 确定性执行壳。
core 不做任何 I/O。纯状态机：喂入(inbound) → 吐出(Vec<Effect>)。
真正的网络/存储/线程由平台驱动层做。
```

**依赖方向（铁律，违反即架构失败）**：

```
helix-driver-native  ─┐
helix-driver-wasm    ─┼─► helix-core ◄─── helix-im
helix-driver-ffi     ─┘                  （及将来所有业务模块）
```

- `helix-core` 不依赖任何驱动层，不知道 tokio/sqlx/reqwest/web-sys/tauri。
- `helix-im` 只依赖 `helix-core`（port trait + module host API + effect types）。
- 驱动层依赖 `helix-core`，实现其 port trait。
- 应用层（Tauri app / WASM JS 胶水 / FFI）把驱动层实现注入 `helix-core` engine。

---

## 1. Ports — trait 签名草案

### 1.0 基础：MaybeSend / MaybeSync（三栖 Send 管理）

必须在 port trait bound 上用 `MaybeSend + MaybeSync`，不写裸 `Send + Sync`，因为 wasm 单线程不可用 `Send`。

```rust
// helix-core/src/platform.rs

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> MaybeSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSend for T {}

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSync: Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Sync> MaybeSync for T {}

#[cfg(target_arch = "wasm32")]
pub trait MaybeSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSync for T {}

/// cfg-gated spawn：模块内部需要 spawn 内部任务时调这个，不直接调 tokio::spawn
pub fn spawn<F>(fut: F)
where
    F: std::future::Future<Output = ()> + 'static,
    #[cfg(not(target_arch = "wasm32"))] F: Send,
{
    #[cfg(not(target_arch = "wasm32"))]
    { tokio::spawn(fut); }
    #[cfg(target_arch = "wasm32")]
    { wasm_bindgen_futures::spawn_local(fut); }
}
```

**为什么必需（对照三栖）**：wasm32-unknown-unknown 无原生线程，所有 Future 均为 `!Send`。若 port trait bound 写 `Send`，wasm 端实现编译失败。`MaybeSend` 用 cfg 把这个差异收敛到 30 行，让所有模块透明使用。

---

### 1.1 Transport Port

**为什么必需**：native 用 tokio-tungstenite（epoll/kqueue reactor），wasm 用 web-sys WebSocket（event-driven callback），ffi 用平台 socket 回调。三端接口完全不同，模块不能直接依赖任何一个。Transport = "搬不透明字节帧"的契约，core 不解析帧内容（14 类 IM type 由 helix-im 解析）。

**async-trait 选型**：Transport 需要 `dyn` 动态分发（驱动层在运行时注入），AFIT（Rust 1.75+）不能直接 dyn-safe，故用 `#[async_trait]`。wasm 侧用 `#[async_trait(?Send)]` 去掉 Send bound。

```rust
// helix-core/src/ports/transport.rs

use bytes::Bytes;
use crate::platform::{MaybeSend, MaybeSync};

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport: MaybeSend + MaybeSync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// 建立连接（幂等：已连接时返回 Ok）
    async fn connect(&mut self) -> Result<(), Self::Error>;

    /// 发送出站帧（不透明字节，helix-core 不解析帧格式）
    async fn send(&mut self, frame: Bytes) -> Result<(), Self::Error>;

    /// 接收一帧入站字节（驱动层 loop 调此方法，喂给 engine.step）
    /// 返回 None 表示连接已关闭
    async fn recv(&mut self) -> Result<Option<Bytes>, Self::Error>;

    /// 关闭连接（幂等）
    async fn close(&mut self) -> Result<(), Self::Error>;
}
```

**三端实现注记**：
- Native：`helix-driver-native` 实现，底层 `tokio-tungstenite::WebSocketStream`，放在 `[target.'cfg(not(target_arch="wasm32"))'.dependencies]`。
- WASM：`helix-driver-wasm` 实现，底层 `web-sys WebSocket` 事件驱动 → `futures_channel::mpsc::unbounded()` 桥接为 async Stream。注意 `JsValue: !Send`，必须在 `spawn_local` 内处理。
- FFI：host 提供 C ABI 回调（`send_fn: extern "C" fn(*const u8, usize) -> i32`，`on_recv: fn(data: *const u8, len: usize)`），用 `futures_channel::mpsc` 桥接为 `recv()` future。回调返回前必须把 C 侧 buffer 拷贝到 Rust 堆。

---

### 1.2 Storage Port

**为什么必需**：native 用 sqlx/rusqlite（OS FS + tokio blocking），wasm 用 sqlite-wasm-rs + OPFS（Worker 初始化），ffi 用平台 SQLite handle C ABI。`sqlx` 永远不能编译到 `wasm32-unknown-unknown`（tokio + mio + getrandom 三重失败，永久限制）。Storage port = "通用 关系/KV 原语"，IM 的表名/字段/ON CONFLICT 键**全是参数**，不进 trait 签名。

**关键：IM 需要的"写成功才推 cursor"契约**（peek/commit 两段式，§6 in demands doc）驱动了 `correlation_id` 设计——`execute_batch` 返回 `CorrelationId`，`PersistResult` 作为入站事件喂回 engine，这是 core 对所有模块的通用契约，不是 IM 专属。

```rust
// helix-core/src/ports/storage.rs

use crate::platform::{MaybeSend, MaybeSync};

/// 通用 SQL 参数（core 不知道具体列名，由模块传参数列表）
#[derive(Clone, Debug)]
pub enum SqlValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
    Bool(bool),
}

/// 通用行：列名 -> 值 的有序列表（保留顺序，与 INSERT 列列表对应）
pub type Row = Vec<(String, SqlValue)>;

/// Persist effect 的相关 id，用于"写成功才推"配对（见 effect.rs CorrelatedPersist）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CorrelationId(pub u64);

/// 通用 upsert 描述：batch_upsert(table, rows, conflict_key)
/// IM 的 ON CONFLICT(temporary_id) DO UPDATE SET ... 全由调用方填参数
#[derive(Clone, Debug)]
pub struct UpsertSpec {
    pub table: String,
    pub rows: Vec<Row>,
    /// ON CONFLICT 列名；None 表示 INSERT OR REPLACE
    pub conflict_key: Option<String>,
}

/// 通用 update 描述：update(table, key_col, key_val, patch)
#[derive(Clone, Debug)]
pub struct UpdateSpec {
    pub table: String,
    pub key_col: String,
    pub key_val: SqlValue,
    pub patch: Row,  // 要修改的列，其余不动
}

/// 单调 cursor upsert：cursor_upsert(scope_key, value)
/// MAX guard 语义：只有 value > 当前值才写入（O(1) upsert）
/// scope_key 由调用方传（IM 传 channel_id 字符串，core 不知道这是 channel）
#[derive(Clone, Debug)]
pub struct CursorUpsertSpec {
    pub scope_key: String,
    pub value: i64,
}

/// 通用 KV get
#[derive(Clone, Debug)]
pub struct GetSpec {
    pub table: String,
    pub key: String,
}

/// Storage port：core 只定义原语签名，不知道 IM 表结构
/// 注：Storage 本身不直接暴露 async 方法给模块调用——
/// 模块通过 effect（Persist variant）请求存储，驱动层兑现后把 PersistResult 喂回 engine。
/// 因此这个 trait 主要由驱动层实现，engine 在执行 Persist effect 时调用。
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Storage: MaybeSend + MaybeSync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;

    /// 批量 upsert（热路径 O(1) per row，冲突键由参数指定）
    async fn batch_upsert(&self, spec: UpsertSpec) -> Result<(), Self::Error>;

    /// 按键更新字段（update content / mark revoked 等，patch 列由参数指定）
    async fn update(&self, spec: UpdateSpec) -> Result<(), Self::Error>;

    /// 批量 update（batch_apply_read 类需求：多行同 patch）
    async fn batch_update(
        &self,
        table: &str,
        key_col: &str,
        key_vals: &[SqlValue],
        patch: Row,
    ) -> Result<(), Self::Error>;

    /// 单调 cursor upsert（MAX guard，O(1)，scope_key 由调用方指定）
    async fn cursor_upsert(&self, spec: CursorUpsertSpec) -> Result<(), Self::Error>;

    /// KV get（表名 + 键 → 值行，Optional）
    async fn get(&self, spec: GetSpec) -> Result<Option<Row>, Self::Error>;

    /// KV set（表名 + 键 + 值行）
    async fn set(&self, table: &str, key: &str, value: Row) -> Result<(), Self::Error>;
}
```

**三端实现注记**：
- Native：`sqlx::SqlitePool` 实现，所有方法用 `sqlx::query!` / `execute`，在 tokio runtime 内直接 await。
- WASM：`sqlite-wasm-rs` + `opfs-sahpool` VFS。注意 OPFS synchronous access handle 只能在 Web Worker 中初始化；main thread 调用需要通过 Worker message bridge 做异步代理。`getrandom = { features = ["js"] }` 必须在 workspace 级声明。
- FFI：host 提供 `execute_fn: extern "C" fn(*const c_char, *const SqlParam, usize) -> i32` 等 C ABI，用 `futures_channel::oneshot` 包装为 Future。

**关于为什么 Storage port 不用泛型而用 dyn（对象安全性选择）**：模块在运行时收到注入的 `Arc<dyn Storage>`（或 `Box<dyn Storage>`），允许在不重编模块的情况下替换驱动实现。这比泛型参数化更适合"模块不知道底层 Storage 类型"的场景。

---

### 1.3 Clock Port

**为什么必需**：`std::time::Instant` 在 `wasm32-unknown-unknown` 上运行时 panic（不是编译错误，是 not supported panic）。`std::time::SystemTime` 同样不可用。Clock 必须是注入 port，不能在 core 里直接调 `std::time`。

**设计裁决**：Clock 不是"模块直接读时钟"——而是模块吐 `ScheduleTimer { after_ms }` effect，驱动层用真实 Clock 兑现，到点把 `TimerFired` 事件喂回 engine。`now_ms` 作为 `engine.step` 的入参，让纯逻辑能给落库行打时间戳而自身不读时钟（确定性来源）。

```rust
// helix-core/src/ports/clock.rs

use crate::platform::MaybeSend;

pub trait Clock: Clone + MaybeSend + 'static {
    /// 返回 Unix epoch 毫秒（u64 到 year 584554 不溢出）
    fn now_ms(&self) -> u64;
}

// --- 测试用 MockClock（三端通用，无平台依赖）---

use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

#[derive(Clone)]
pub struct MockClock {
    inner: Arc<AtomicU64>,
}

impl MockClock {
    pub fn new(initial_ms: u64) -> Self {
        Self { inner: Arc::new(AtomicU64::new(initial_ms)) }
    }

    /// 在测试中手动推进时间（确定性回放的关键）
    pub fn advance(&self, delta_ms: u64) {
        self.inner.fetch_add(delta_ms, Ordering::Relaxed);
    }
}

impl Clock for MockClock {
    fn now_ms(&self) -> u64 {
        self.inner.load(Ordering::Relaxed)
    }
}
```

**三端实现**：
- Native：`SystemTimeClock`，`SystemTime::UNIX_EPOCH.elapsed().unwrap().as_millis() as u64`
- WASM：`JsClock`，`js_sys::Date::now() as u64`（或 `performance.timeOrigin + performance.now()`）
- FFI：`FfiClock`，host 注入 `fn() -> u64` 函数指针
- 所有端测试：`MockClock`（上面，三端均编译）

---

### 1.4 EventSink Port

**为什么必需**：模块吐出的 DomainEvent 需要送到宿主 UI（Tauri: `app.emit`，WASM: JS callback/postMessage，FFI: C 回调）。三端完全不同，不能在 core 里写死。`im:` 前缀、`im:__bus__` wire 名——这是 adapter 序列化时的事，core 只搬强类型 `DomainEvent` enum。

**设计裁决**：EventSink 是同步 trait（`emit` 不返回 Future）。原因：模块在纯逻辑执行时吐 `Emit(event)` effect，驱动层收到后可以选择异步投递，但 core 侧的"登记意图"是同步的，保持状态机纯洁。

```rust
// helix-core/src/ports/event_sink.rs

use crate::platform::{MaybeSend, MaybeSync};
use crate::event::DomainEvent;

pub trait EventSink: MaybeSend + MaybeSync + 'static {
    /// 发布一个领域事件给宿主（同步登记意图，驱动层自行决定投递时机）
    fn emit(&self, event: DomainEvent);
}

// 测试用 CapturingEventSink
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct CapturingEventSink {
    events: Arc<Mutex<Vec<DomainEvent>>>,
}

impl CapturingEventSink {
    pub fn drain(&self) -> Vec<DomainEvent> {
        self.events.lock().unwrap().drain(..).collect()
    }
}

impl EventSink for CapturingEventSink {
    fn emit(&self, event: DomainEvent) {
        self.events.lock().unwrap().push(event);
    }
}
```

**关于 EventBus channel 形态**：

core 的 EventSink 是"推送给宿主"的出站口，不是模块间通信总线。模块间通信（如果需要）走 `InboundEvent` 路径（一个模块的输出作为下一个模块的入站喂给 engine.step）。

若驱动层需要 fan-out（多个 UI 订阅者），在驱动层实现：
- Native：`tokio::sync::broadcast` 作为 EventSink 的背后实现，处理 `lagged` 错误。
- WASM：JS `EventTarget` / `BroadcastChannel`。
- 选型原则：`broadcast` 适合 fan-out + 允许 lagged；`mpsc` 适合背压管道；`watch` 适合最新值语义。这些选择在**驱动层**，不在 core。

---

### 1.5 IdSource Port

**为什么必需**：确定性来源。IM 主路径 temporaryId 由 UI 注入（源码印证），但模块偶尔需要自己生成临时键（如 CorrelationId 生成、doc 分支）。在 wasm 上 `getrandom` 必须启用 `js` feature，在 FFI 上需要平台熵源。IdSource port 让模块不直接调 `rand::thread_rng()`，支持测试时注入确定性序列（回放）。

```rust
// helix-core/src/ports/id_source.rs

use crate::platform::MaybeSend;

pub trait IdSource: Clone + MaybeSend + 'static {
    fn next_id(&self) -> u64;
}

// 测试用确定性序列 IdSource
use std::sync::{Arc, atomic::{AtomicU64, Ordering}};

#[derive(Clone)]
pub struct SequentialIdSource {
    counter: Arc<AtomicU64>,
}

impl SequentialIdSource {
    pub fn new(start: u64) -> Self {
        Self { counter: Arc::new(AtomicU64::new(start)) }
    }
}

impl IdSource for SequentialIdSource {
    fn next_id(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}
```

---

### 1.6 HttpRequester Port

**为什么必需**：IM 发送主路径走 HTTP POST（不走 WS），gate fire 也走 HTTP（`/sync/notify`）。Transport（WS）与 HttpRequester（HTTP）是两个 port，因为两条路径并存且三端实现不同（native=reqwest，wasm=gloo-net/fetch，ffi=平台 http 栈）。core 不知道 URL、query params、body schema——这些由 helix-im 填入 `HttpRequest` 参数结构体。

```rust
// helix-core/src/ports/http_requester.rs

use bytes::Bytes;
use std::collections::HashMap;
use crate::platform::{MaybeSend, MaybeSync};

#[derive(Clone, Debug)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

/// 不透明 HTTP 请求（URL/header/body 由模块填，core 不解析）
#[derive(Clone, Debug)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Option<Bytes>,
}

/// 不透明 HTTP 响应
#[derive(Clone, Debug)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Bytes,
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait HttpRequester: MaybeSend + MaybeSync + 'static {
    type Error: std::error::Error + Send + Sync + 'static;
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, Self::Error>;
}
```

**三端实现**：
- Native：`reqwest::Client` 实现，放 `[target.'cfg(not(wasm32))'.dependencies]`。
- WASM：`gloo-net::http::Request` 或 `web-sys fetch`（reqwest 有 wasm fetch backend，但无请求流式、无自定义 TLS）。
- FFI：host 提供 C ABI HTTP callback，包装为 oneshot Future。

---

## 2. Module 契约（业务模块怎么定义）

### 2.1 设计原则

核心约束：
- 模块**不做任何 I/O**，不 await 任何 I/O future，不调 port 的 async 方法。
- 模块的**所有输出**通过 `Vec<Effect>` 表达，驱动层兑现 effect 后把结果喂回 `engine.step`。
- 模块的**内部状态**跨 step 保存（模块 struct 持有 state）。
- `handle()` 的返回是 `Result<Vec<Effect>, ModuleError>`——**同步，无 async**。这是确定性的关键。

**为什么 handle 是同步的（无 async）**：sans-IO 范式要求 core 状态机纯函数化。如果 handle 是 async，它就能在内部 await I/O，破坏可回放性。所有 I/O 必须外置为 effect，由驱动层 async 兑现后再喂回。

### 2.2 Module trait

```rust
// helix-core/src/module_host.rs

use crate::effect::Effect;
use crate::engine::InboundEvent;
use crate::error::ModuleError;

/// 业务模块契约
/// 实现者：helix-im、将来 helix-presence、helix-notify...
pub trait Module: 'static {
    /// 模块唯一名称（用于 ModuleHost 路由 + 日志）
    fn name(&self) -> &'static str;

    /// 判断此模块是否处理该入站事件
    /// 快速路径：纯比较，不做任何 I/O，不修改 self
    fn accepts(&self, event: &InboundEvent) -> bool;

    /// 处理入站事件，吐出 effect 列表
    /// 严格同步（no async），不做任何 I/O，不调 port async 方法
    /// 可修改 self（模块内部状态机推进）
    fn handle(&mut self, event: InboundEvent) -> Result<Vec<Effect>, ModuleError>;

    /// 生命周期：模块启动（初始化内部状态，可吐出初始 effect 如"拉取初始游标"）
    fn on_start(&mut self) -> Vec<Effect> {
        vec![]
    }

    /// 生命周期：模块暂停（如 app 进入后台，可吐出"停止计时器"effect）
    fn on_suspend(&mut self) -> Vec<Effect> {
        vec![]
    }

    /// 生命周期：模块恢复
    fn on_resume(&mut self) -> Vec<Effect> {
        vec![]
    }
}
```

### 2.3 ModuleHost（注册 + 路由）

```rust
// helix-core/src/module_host.rs（续）

use crate::error::CoreError;

pub struct ModuleHost {
    modules: Vec<Box<dyn Module>>,
}

impl ModuleHost {
    pub fn new() -> Self {
        Self { modules: Vec::new() }
    }

    /// 注册业务模块（顺序即优先级；先注册先匹配）
    pub fn register<M: Module>(&mut self, module: M) {
        self.modules.push(Box::new(module));
    }

    /// 分发入站事件：找到第一个 accepts 的模块，调其 handle
    /// 若无模块接受 → CoreError::NoHandler（非致命，记 trace 即可）
    pub(crate) fn dispatch(
        &mut self,
        event: InboundEvent,
    ) -> Result<Vec<Effect>, CoreError> {
        for module in &mut self.modules {
            if module.accepts(&event) {
                return module.handle(event).map_err(CoreError::Module);
            }
        }
        tracing::trace!("no handler for event: {:?}", event.kind());
        Err(CoreError::NoHandler)
    }

    pub(crate) fn start_all(&mut self) -> Vec<Effect> {
        self.modules.iter_mut().flat_map(|m| m.on_start()).collect()
    }

    pub(crate) fn suspend_all(&mut self) -> Vec<Effect> {
        self.modules.iter_mut().flat_map(|m| m.on_suspend()).collect()
    }

    pub(crate) fn resume_all(&mut self) -> Vec<Effect> {
        self.modules.iter_mut().flat_map(|m| m.on_resume()).collect()
    }
}
```

**加新业务模块 = 实现 Module trait + 调 `host.register(MyModule::new(...))`，不改 core 任何代码**。这是开闭原则在架构层的体现。

---

## 3. Effect / Event 模型

### 3.1 InboundEvent（喂入执行壳的全部来源）

```rust
// helix-core/src/engine.rs

use bytes::Bytes;
use crate::storage::{Row, CorrelationId};

/// 执行壳的输入——所有外部事件统一入口
/// 驱动层负责把网络/存储/定时器/用户意图包装成 InboundEvent 喂给 engine.step
#[derive(Debug)]
pub enum InboundEvent {
    /// 来自 Transport 的入站字节帧（helix-im 内部解析为 14 类 type；core 不解析）
    Frame(Bytes),

    /// 来自 HttpRequester 的响应（correlation_id 匹配对应的 HttpRequest effect）
    HttpResponse {
        correlation_id: CorrelationId,
        status: u16,
        body: Bytes,
        ok: bool,
    },

    /// 持久化操作完成回报（"写成功才推 cursor"的关键入站事件）
    PersistResult {
        correlation_id: CorrelationId,
        ok: bool,
        /// 失败原因（ok=false 时填，供模块决策是否重拉/标错）
        error_code: Option<u32>,
    },

    /// 定时器触发（ScheduleTimer effect 到期后由驱动层喂回）
    TimerFired {
        timer_id: u64,
    },

    /// 宿主注入的时钟（每次 step 携带，让模块打时间戳而不读系统时钟）
    /// 通常不单独作为一个事件，而是 engine.step 的参数；
    /// 这里保留作为"clock tick"触发时用（如定期心跳场景）
    ClockTick { now_ms: u64 },

    /// 应用层下发命令（用户意图：send / read / resend 等）
    /// 载荷类型由模块自己定义（用 Any + 下转型，或 serde_json::Value）
    /// 推荐：用 serde_json::Value，避免跨 crate 的 Any 转型耦合
    AppCommand {
        module: &'static str,
        payload: serde_json::Value,
    },

    /// 生命周期事件
    LifecycleStart,
    LifecycleSuspend,
    LifecycleResume,
}

impl InboundEvent {
    /// 供 module.accepts() 快速判断事件类别
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Frame(_) => "frame",
            Self::HttpResponse { .. } => "http_response",
            Self::PersistResult { .. } => "persist_result",
            Self::TimerFired { .. } => "timer_fired",
            Self::ClockTick { .. } => "clock_tick",
            Self::AppCommand { module, .. } => module,
            Self::LifecycleStart => "lifecycle_start",
            Self::LifecycleSuspend => "lifecycle_suspend",
            Self::LifecycleResume => "lifecycle_resume",
        }
    }
}
```

### 3.2 DomainEvent（模块吐出给宿主 UI 的领域事件）

```rust
// helix-core/src/event.rs

/// 强类型 DomainEvent enum
/// helix-im 定义自己的 variants；将来 helix-presence 添加自己的 variants
/// core 搬这个 enum 的实例给 EventSink，不解析 variant 内容
/// 
/// 设计：用 tagged union 而非 Box<dyn Any>——保持 make-invalid-states-unrepresentable
/// 代价：添加新业务模块需要在这个 enum 添加 variant（可接受，业务模块数量有限）
/// 若将来模块数爆炸，可改为 Box<dyn Any + Send + Sync> + 模块自注册序列化器
#[derive(Debug, Clone)]
pub enum DomainEvent {
    /// IM 消息收到（实时 echo 或 sync Events 到达）
    Im(ImDomainEvent),
    // 将来：Presence(PresenceDomainEvent), Notify(NotifyDomainEvent), ...
}

/// IM 模块领域事件（对应源码 emit_im 的全集）
/// wire 名（`im:post:received` 等）在 helix-driver-native/wasm/ffi 的 adapter 里序列化
#[derive(Debug, Clone)]
pub enum ImDomainEvent {
    PostReceived    { channel_id: String, message_id: String },
    PostUpdated     { channel_id: String, message_id: String },
    PostDeleted     { channel_id: String, message_id: String },
    PostBatchUpdated { channel_id: String, message_ids: Vec<String> },
    PostRead        { channel_id: String, last_read_seq: i64 },
    ChannelReadEcho { channel_id: String, last_read_seq: i64 },
    ChannelUpdateByPost { channel_id: String },
    SyncTooLong     { channel_id: String, reset_to: i64 },
}
```

**关于 DomainEvent enum 扩展性**：用显式 tagged union（不用 `Box<dyn Any>`）的理由是：
1. 编译时穷举——所有事件处理都在编译时检查，不能漏处理 variant。
2. `Clone + Debug`——测试容易，序列化容易。
3. 代价是添加新模块需要在这里加 variant，但业务模块数量有限（不是"任意第三方插件"场景）。可接受。

### 3.3 Effect（模块的全部输出）

这是核心数据结构。Effect 是 core 对"模块意图"的统一表达，驱动层逐个兑现。

```rust
// helix-core/src/effect.rs

use bytes::Bytes;
use crate::event::DomainEvent;
use crate::storage::{UpsertSpec, UpdateSpec, CursorUpsertSpec, GetSpec, CorrelationId, Row, SqlValue};
use crate::ports::http_requester::HttpRequest;

/// 所有 effect variant 的通用 correlation id wrapper
/// 携带 correlation_id 的 effect 在完成时会有对应 InboundEvent::PersistResult 或 HttpResponse 回报
/// "写成功才推 cursor"靠这个机制实现（§6 of demands doc）
#[derive(Debug, Clone)]
pub struct Correlated<T> {
    pub correlation_id: CorrelationId,
    pub payload: T,
}

/// 持久化 effect payload（多种存储操作）
#[derive(Debug, Clone)]
pub enum PersistOp {
    BatchUpsert(UpsertSpec),
    Update(UpdateSpec),
    BatchUpdate {
        table: String,
        key_col: String,
        key_vals: Vec<SqlValue>,
        patch: Row,
    },
    CursorUpsert(CursorUpsertSpec),
    Set { table: String, key: String, value: Row },
}

/// Effect —— 模块吐出的全部输出类型
#[derive(Debug, Clone)]
pub enum Effect {
    /// 持久化请求（携带 correlation_id 以便"写成功才推 cursor"配对）
    /// 驱动层执行后必须把 PersistResult { correlation_id, ok } 喂回 engine
    Persist(Correlated<PersistOp>),

    /// 持久化请求（fire-and-forget，不需要成功回报）
    /// 用于不需要"写成功才推"的存储操作（如非关键日志）
    PersistFire(PersistOp),

    /// 出站帧（→ Transport，不透明字节）
    SendFrame(Bytes),

    /// 出站 HTTP 请求（携带 correlation_id，响应作为 HttpResponse 入站事件回报）
    SendHttp(Correlated<HttpRequest>),

    /// 领域事件（→ EventSink → 宿主 UI）
    Emit(DomainEvent),

    /// 请求定时器（驱动层在 after_ms 后把 TimerFired{timer_id} 喂回 engine）
    ScheduleTimer { timer_id: u64, after_ms: u64 },

    /// 撤销定时器（如发送超时被 echo 提前撤）
    CancelTimer { timer_id: u64 },

    /// 从 Storage 读取（驱动层执行后把结果... 目前通过下一个 AppCommand 或专用 InboundEvent 回报）
    /// 注：Read 是唯一不带 correlation_id 的异步 effect——
    /// 因为读通常在模块的"初始化"或"查询"路径，返回值通过 InboundEvent::AppCommand 回喂
    /// 这个设计保持了写路径（Persist）与读路径的清晰分离
    ReadRequest { get_spec: GetSpec, reply_module: &'static str },
}
```

**Effect 设计决策说明**：

1. **`Persist` vs `PersistFire`**：分离有 correlation_id（需成功回报）和 fire-and-forget 两种持久化。IM 的"写成功才推 cursor"走 `Persist`，普通落库（不影响 cursor）走 `PersistFire`。

2. **Correlation id 是通用机制**：不是 IM 专属。任何需要"异步操作完成后再做下一步"的模块都可以用。

3. **`Emit` 是同步的**：EventSink.emit 是同步的（驱动层登记后自己异步投递）。模块吐 `Emit` 不等待。

4. **`ScheduleTimer` 的 `timer_id` 由模块生成**（用 IdSource port 生成，或模块内维护计数器）。CancelTimer 用同一 id。

---

### 3.4 IM 的"写成功才推 cursor"在 Effect 模型里的体现

两段式 step 完整流程：

```
Step N 收到 sync Events 入站：
  模块内核 peek_up_to（纯逻辑，算出该写哪些 + target_seq）
  → 吐出：
      Persist { corr=X, BatchUpsert(events分桶) }   ← 先写数据
      // 注意：此时不吐 CursorUpsert
  → 模块内部状态保存 pending_commit[X] = target_seq

Step N+1 收到 PersistResult { corr=X, ok=true } 入站：
  模块内核 commit_up_to（纯逻辑）
  → 吐出：
      PersistFire { CursorUpsert { scope_key=channel_id, value=target_seq } }
      // MAX guard 由 Storage 通用原语保证单调性
  → 内存 cursor 推进

若 ok=false：
  模块内核丢弃 pending_commit[X]，不推 cursor
  → 下次 sync 仍从旧 cursor 拉 → 不丢事件
```

这个流程完全在模块内核（helix-im）的同步 `handle()` 方法里表达，没有任何 I/O，没有 async，可以在三端一致地回放。

---

## 4. Sans-IO 确定性执行壳

### 4.1 CoreEngine 设计

**设计约束**：
- `step()` 是**同步**的（无 async），接受 `InboundEvent + now_ms`，产出 `Vec<Effect>`。
- 驱动层拿到 effects 后执行 I/O，I/O 完成后把结果包成下一个 `InboundEvent` 喂回。
- 整个循环不在 core 里——core 只提供单次 `step`，循环在驱动层的事件泵里。

```rust
// helix-core/src/engine.rs（续）

use crate::module_host::ModuleHost;
use crate::error::CoreError;

/// 运行时注入的上下文（每次 step 传入，不缓存在 engine 内）
pub struct StepContext {
    /// host 注入的当前时间（毫秒，确定性来源——不在 core 内读系统时钟）
    pub now_ms: u64,
}

pub struct CoreEngine {
    host: ModuleHost,
}

impl CoreEngine {
    pub fn new(host: ModuleHost) -> Self {
        Self { host }
    }

    /// 启动：让所有模块初始化，返回初始 effect 列表
    pub fn start(&mut self) -> Vec<Effect> {
        self.host.start_all()
    }

    /// 暂停（app 进入后台）
    pub fn suspend(&mut self) -> Vec<Effect> {
        self.host.suspend_all()
    }

    /// 恢复
    pub fn resume(&mut self) -> Vec<Effect> {
        self.host.resume_all()
    }

    /// 核心：单步执行
    /// 严格同步，无 async，无 I/O，无 tokio，无 web-sys
    /// 输入：一个入站事件 + 当前时间（host 注入）
    /// 输出：这一步产生的所有 effect（驱动层负责兑现）
    pub fn step(
        &mut self,
        event: InboundEvent,
        ctx: &StepContext,
    ) -> Result<Vec<Effect>, CoreError> {
        // 生命周期路由直接在 engine 层处理（不经模块分发）
        match &event {
            InboundEvent::LifecycleStart   => return Ok(self.host.start_all()),
            InboundEvent::LifecycleSuspend => return Ok(self.host.suspend_all()),
            InboundEvent::LifecycleResume  => return Ok(self.host.resume_all()),
            _ => {}
        }

        self.host.dispatch(event)
    }
}
```

**StepContext 携带 `now_ms`**：不是 Clock trait 直接注入 engine——而是每次 step 外部传 `now_ms`。原因：

1. 测试时可以精确控制每次 step 的时间（`MockClock`）。
2. 驱动层负责读真实 Clock（`clock.now_ms()`）然后传进来，engine 和模块完全不读时钟。
3. 批处理回放时，可以用历史 now_ms 序列驱动，精确复现历史执行。

### 4.2 驱动层事件泵（示意，不在 core 里）

```rust
// helix-driver-native/src/event_pump.rs（示意，不是 core 代码）

// 驱动层的事件泵：
// 1. 从 Transport 接收帧 → 包成 InboundEvent::Frame → engine.step
// 2. 执行 engine.step 返回的 Effect 列表（异步 I/O）
// 3. I/O 完成 → 把结果包成 InboundEvent（PersistResult / HttpResponse / TimerFired）→ engine.step
// 4. 循环（Tokio event loop 或 wasm-bindgen-futures spawn_local）

async fn event_pump(engine: &Mutex<CoreEngine>, transport: &mut dyn Transport, ...) {
    loop {
        tokio::select! {
            frame = transport.recv() => {
                let event = InboundEvent::Frame(frame?);
                let effects = engine.lock().step(event, &StepContext { now_ms: clock.now_ms() })?;
                execute_effects(effects, ...).await;
            }
            // timers, app commands, etc.
        }
    }
}
```

**wasm32 上的事件泵**：`spawn_local(async { ... })` 驱动，`gloo-timers` 实现 ScheduleTimer，`web-sys WebSocket.onmessage` 回调 → `futures_channel::mpsc` → `recv()` future poll。

### 4.3 可回放性 / 可测试性保证

由以下四个约束共同保证：

1. `step()` 是**纯同步函数**（给定相同 `InboundEvent` 序列 + 相同 `now_ms` 序列 → 相同 `Vec<Effect>` 序列），无随机、无真实 I/O。
2. `now_ms` 由外部注入（测试用 `MockClock`，回放用历史 `now_ms` 序列）。
3. `IdSource` port 注入（测试用 `SequentialIdSource`，回放用记录的 id 序列）。
4. 所有 I/O 在 effect 里表达（外置），不在模块内部 await。

**单元测试模式**（三端通用，无 tokio 依赖）：

```rust
#[test]
fn test_send_echo_roundtrip_converges() {
    let clock = MockClock::new(1_000_000);
    let id_src = SequentialIdSource::new(1);
    let event_sink = CapturingEventSink::default();

    let im_module = ImModule::new(clock.clone(), id_src.clone(), event_sink.clone());
    let mut host = ModuleHost::new();
    host.register(im_module);
    let mut engine = CoreEngine::new(host);

    // 1. 用户发消息
    let effects = engine.step(
        InboundEvent::AppCommand {
            module: "im",
            payload: serde_json::json!({
                "action": "send",
                "channel_id": "ch1",
                "temporary_id": "tmp-001",
                "content": "hello",
            }),
        },
        &StepContext { now_ms: clock.now_ms() },
    ).unwrap();

    // 断言：应吐出 PersistFire（乐观落库）+ ScheduleTimer（15s）+ SendFrame（WS 发出）
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistFire(_))));
    assert!(effects.iter().any(|e| matches!(e, Effect::ScheduleTimer { after_ms: 15000, .. })));
    assert!(effects.iter().any(|e| matches!(e, Effect::SendFrame(_))));

    // 2. echo 到达（模拟服务端回声，带 server id）
    let corr_id = /* 从第一步的 Persist effect 拿到 correlation_id */;
    clock.advance(100); // echo 在 100ms 内到达
    let effects2 = engine.step(
        InboundEvent::Frame(/* 模拟 echo frame bytes */),
        &StepContext { now_ms: clock.now_ms() },
    ).unwrap();

    // 断言：应吐出 CancelTimer + Persist（by temporaryId 覆盖）+ Emit(PostReceived)
    assert!(effects2.iter().any(|e| matches!(e, Effect::CancelTimer { .. })));
    assert!(effects2.iter().any(|e| matches!(e, Effect::Emit(DomainEvent::Im(ImDomainEvent::PostReceived { .. })))));

    // 3. PersistResult 回报（模拟落库成功）
    let persist_corr = /* 从 effects2 的 Persist 拿 corr */;
    let effects3 = engine.step(
        InboundEvent::PersistResult { correlation_id: persist_corr, ok: true, error_code: None },
        &StepContext { now_ms: clock.now_ms() },
    ).unwrap();

    // 断言：cursor 推进（CursorUpsert effect）
    assert!(effects3.iter().any(|e| matches!(e, Effect::PersistFire(PersistOp::CursorUpsert(_)))));

    // EventSink 应已收到 PostReceived 事件
    let emitted = event_sink.drain();
    assert!(emitted.iter().any(|e| matches!(e, DomainEvent::Im(ImDomainEvent::PostReceived { .. }))));
}
```

---

## 5. Cargo workspace + feature/cfg gating 策略

### 5.1 workspace Cargo.toml

```toml
# /workspace/rustWorkspace/helix/Cargo.toml

[workspace]
members = [
    "crates/helix-core",
    "crates/helix-im",
    "crates/helix-driver-native",
    "crates/helix-driver-wasm",
    "crates/helix-driver-ffi",
]
resolver = "2"

[workspace.dependencies]
# Core 依赖（三端均可编译）
serde        = { version = "1", features = ["derive"] }
serde_json   = { version = "1" }
thiserror    = "2"
bytes        = "1"
async-trait  = "0.1"
futures-core = "0.3"
futures-channel = "0.3"
tracing      = "0.1"

# wasm32 上必须在 workspace 级声明 getrandom js feature
# 不只放在 helix-driver-wasm，因为 helix-core 的传递依赖（如 uuid）也需要此 feature
# native 上此声明无害（getrandom 使用 OS entropy，js feature 被 cfg 排除）
getrandom    = { version = "0.2", features = ["js"] }

[profile.release]
opt-level     = "z"
lto           = true
codegen-units = 1
panic         = "abort"    # wasm 上 unwind 不可用；native 上省 panic machinery
strip         = true

[profile.dev]
# dev 不要 opt-level = 0（sqlx 等在 debug 很慢），用 1
opt-level = 1
```

### 5.2 helix-core Cargo.toml

```toml
# crates/helix-core/Cargo.toml

[package]
name    = "helix-core"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib"]

[dependencies]
serde        = { workspace = true }
serde_json   = { workspace = true }
thiserror    = { workspace = true }
bytes        = { workspace = true }
async-trait  = { workspace = true }
futures-core = { workspace = true }
futures-channel = { workspace = true }
tracing      = { workspace = true }

# platform.rs 的 spawn 函数需要在 native 侧调 tokio::spawn
# 放在 target cfg 下（不是 feature），让 wasm 侧不引入 tokio
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio = { version = "1", features = ["rt", "sync"] }  # 最小 tokio subset（仅 spawn/sync）

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen-futures = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["rt", "macros", "sync"] }
```

**关键设计决定**：`helix-core` 对 tokio 的依赖**仅限于 `platform::spawn` 函数**，且只在 `cfg(not(wasm32))` 下。整个 port trait / engine / effect 模型不碰 tokio。这是"零运行时依赖"在实践中的精确边界——`spawn` 函数是个桥接帮助器，不是 I/O 能力。

### 5.3 helix-im Cargo.toml

```toml
# crates/helix-im/Cargo.toml

[package]
name    = "helix-im"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib"]

[dependencies]
helix-core   = { path = "../helix-core" }
serde        = { workspace = true }
serde_json   = { workspace = true }
thiserror    = { workspace = true }
bytes        = { workspace = true }
# helix-im 实现 Module trait，不引入任何 I/O crate
# 所有 I/O 通过 Effect 和 port trait 外置
```

### 5.4 helix-driver-native Cargo.toml

```toml
# crates/helix-driver-native/Cargo.toml

[package]
name    = "helix-driver-native"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["rlib"]

[dependencies]
helix-core = { path = "../helix-core" }
helix-im   = { path = "../helix-im" }
bytes      = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }
tracing    = { workspace = true }

# 平台 I/O crate —— 全部在 cfg(not(wasm32)) 下
[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio             = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "net"] }
sqlx              = { version = "0.8", default-features = false, features = ["sqlite", "runtime-tokio", "macros"] }
tokio-tungstenite = { version = "0.23", features = ["native-tls"] }
reqwest           = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
```

### 5.5 helix-driver-wasm Cargo.toml

```toml
# crates/helix-driver-wasm/Cargo.toml

[package]
name    = "helix-driver-wasm"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]   # wasm 输出为 .wasm

[dependencies]
helix-core = { path = "../helix-core" }
helix-im   = { path = "../helix-im" }
bytes      = { workspace = true }
serde      = { workspace = true }
serde_json = { workspace = true }

[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen         = "0.2"
wasm-bindgen-futures = "0.4"
js-sys               = "0.3"
web-sys              = { version = "0.3", features = [
    "WebSocket", "MessageEvent", "CloseEvent", "ErrorEvent",
    "Performance", "Window",
] }
serde-wasm-bindgen   = "0.6"
gloo-timers          = { version = "0.3", features = ["futures"] }
sqlite-wasm-rs       = "0.3"   # Storage 实现：sqlite-wasm-rs + OPFS sahpool
getrandom            = { workspace = true }
```

### 5.6 helix-driver-ffi Cargo.toml

```toml
# crates/helix-driver-ffi/Cargo.toml

[package]
name    = "helix-driver-ffi"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "staticlib"]   # .so/.dll + .a for Swift/Kotlin

[dependencies]
helix-core = { path = "../helix-core" }
helix-im   = { path = "../helix-im" }
bytes      = { workspace = true }
serde_json = { workspace = true }

[target.'cfg(not(target_arch = "wasm32"))'.dependencies]
tokio          = { version = "1", features = ["rt-multi-thread", "macros"] }
futures-channel = { workspace = true }
# cbindgen 生成 C header；uniffi 用于 Swift/Kotlin 绑定
```

**`panic = "abort"` 与 FFI 安全**：FFI 边界必须不能 unwind。`panic = "abort"` 在 profile.release 里全局设置，所有 cdylib/staticlib 输出都走 abort。如果 dev profile 仍需 unwind（测试），在 FFI 导出函数里 wrap `std::panic::catch_unwind`。

---

## 6. 依赖倒置图

### 6.1 完整依赖方向图

```
┌─────────────────────────────────────────────────────────────────┐
│                     应用层 / 宿主                                 │
│  Tauri App   |   WASM JS 胶水   |   iOS/Android FFI 调用方       │
└──────┬───────────────┬───────────────────┬──────────────────────┘
       │               │                   │
       ▼               ▼                   ▼
┌──────────────┐ ┌─────────────────┐ ┌──────────────────┐
│helix-driver  │ │helix-driver-wasm│ │helix-driver-ffi  │
│-native       │ │                 │ │                  │
│(tokio/sqlx/  │ │(wasm-bindgen/   │ │(cbindgen/uniffi/ │
│tungstenite/  │ │web-sys/sqlite-  │ │C ABI callbacks)  │
│reqwest)      │ │wasm-rs/gloo)    │ │                  │
└──────┬───────┘ └────────┬────────┘ └────────┬─────────┘
       │  实现 port traits │  实现 port traits  │ 实现 port traits
       └───────────────────┼────────────────────┘
                           │
                           ▼
              ┌────────────────────────┐
              │       helix-core        │  ← 产品本体，永不依赖驱动
              │                         │
              │  ports:                 │
              │    Transport trait      │
              │    Storage trait        │
              │    Clock trait          │
              │    EventSink trait      │
              │    IdSource trait       │
              │    HttpRequester trait  │
              │                         │
              │  engine:                │
              │    CoreEngine::step()   │
              │    InboundEvent enum    │
              │    Effect enum          │
              │    CorrelationId        │
              │                         │
              │  module_host:           │
              │    Module trait         │
              │    ModuleHost           │
              │                         │
              │  event:                 │
              │    DomainEvent enum     │
              │                         │
              │  platform:              │
              │    MaybeSend/MaybeSync  │
              │    spawn()              │
              └────────────┬────────────┘
                           │ 依赖（只用 port trait）
                           ▼
              ┌────────────────────────┐
              │       helix-im          │  ← 第一个业务模块
              │  impl Module trait      │
              │  IM 状态机（纯逻辑）    │
              │  DomainEvent::Im 定义   │
              │  零 I/O，零 tokio       │
              └────────────────────────┘
              （将来：helix-presence, helix-notify ...）
```

### 6.2 依赖倒置如何保证三栖

- 驱动层**实现** port trait，但 port trait 在 `helix-core` 里定义。
- `helix-im` 只见到 `helix-core` 的 trait，不知道哪个驱动在背后。
- 三端可以完全独立地替换整个驱动层，而 `helix-core` 和 `helix-im` 的代码完全不变。
- 这是六边形架构（ports & adapters）在 Rust crate 层级的精确实现。

### 6.3 为什么这 6 个 port 是充分必要的（IM 验证）

删掉任何一个 port，IM 就跑不通：

| 删掉 | IM 跑不通的原因 |
|------|------|
| Transport | WS 入站/出站帧无法传递，消息收发断路 |
| Storage | 落库/cursor 无法持久化，离线 sync 无法实现 |
| Clock | 15s 超时/1s gate 无法计时，发送状态机卡死 |
| EventSink | PostReceived 等事件无法推给 UI，界面不更新 |
| IdSource | CorrelationId 生成无法做到确定性（测试/回放失败） |
| HttpRequester | `/posts/create` 直发路径断路，消息无法发出 |

删掉任何一个，等价于 IM 某个完整功能链路失效。

---

## 7. 关键设计取舍备忘

### 7.1 async-trait vs AFIT 取舍

**结论**：port trait 用 `#[async_trait]` + `#[cfg_attr(?Send)]`，core 内部静态路径用 AFIT。

理由：port trait 需要 `dyn Trait`（运行时注入驱动实现），AFIT（Rust 1.75+）不支持直接 dyn。`async-trait` 生成 `Pin<Box<dyn Future>>` 实现对象安全。`?Send` 变体通过 `cfg_attr` 处理 wasm 单线程。

### 7.2 dyn vs 泛型静态分发

**结论**：port 参数用 `dyn`（运行时注入），模块内部调用用泛型（if needed）。

port trait 选 `dyn` 的原因：驱动实现在应用层注入，编译时模块不知道具体类型。用泛型会让 `helix-im` 泛型参数化到具体驱动类型，破坏模块与驱动的解耦。

`dyn Trait` 要求 object safety（无泛型方法，无 `Self` 返回）——当前 port trait 设计均满足。

### 7.3 DomainEvent enum 扩展性取舍

**结论**：显式 tagged union（而非 `Box<dyn Any>`），接受"加新模块需加 variant"的代价。

理由：类型驱动 make-invalid-states-unrepresentable，`Clone + Debug` 测试友好，编译时穷举保证。业务模块数量有限（不是任意第三方插件场景），代价可接受。

若将来模块数量爆炸（>10 个），可以引入 `Box<dyn EventPayload>` + 模块自注册序列化器，但这是到时候再挣的扩展性，不预建。

### 7.4 Storage port 同步 vs 异步的分层

**结论**：Storage port 是 async trait（驱动层实现异步 DB 操作），但模块**不直接调** Storage port——模块通过 effect（Persist）表达意图，驱动层收到 effect 后才调 Storage port。

这个分层保证：模块的 `handle()` 方法是同步的，可以在三端无差异地作为纯函数执行。

### 7.5 futures-channel vs tokio::sync

**结论**：core 里所有 channel 用 `futures-channel`，不用 `tokio::sync`。

`futures-channel` 无 executor 依赖，在 `wasm32-unknown-unknown` 可编译。`tokio::sync::broadcast/mpsc` 依赖 tokio reactor，只进驱动层。

---

## 8. 文件结构骨架

```
crates/helix-core/src/
├── lib.rs              # pub use 各模块，feature gates
├── platform.rs         # MaybeSend / MaybeSync / spawn()
├── error.rs            # CoreError enum, ModuleError
├── engine.rs           # CoreEngine, InboundEvent, StepContext
├── effect.rs           # Effect enum, Correlated<T>, PersistOp
├── event.rs            # DomainEvent, ImDomainEvent
├── module_host.rs      # Module trait, ModuleHost
└── ports/
    ├── mod.rs
    ├── transport.rs    # Transport trait
    ├── storage.rs      # Storage trait, SqlValue, Row, UpsertSpec, ...
    ├── clock.rs        # Clock trait, MockClock
    ├── event_sink.rs   # EventSink trait, CapturingEventSink
    ├── id_source.rs    # IdSource trait, SequentialIdSource
    └── http_requester.rs # HttpRequester trait, HttpRequest, HttpResponse

crates/helix-im/src/
├── lib.rs
├── module.rs           # impl Module for ImModule
├── state.rs            # ImState（内存状态机：pending_commits, cursor, gate buffer）
├── handler/
│   ├── mod.rs
│   ├── inbound_frame.rs # 解析 Frame → IM 14 类 type
│   ├── app_command.rs   # 处理 send/read/resend
│   ├── persist_result.rs # 处理 PersistResult（peek/commit 两段式）
│   └── timer_fired.rs  # 处理 TimerFired（15s/1s）
└── effects.rs          # IM 特定 effect 构造器（不暴露给 core）
```

---

## 9. 污染红线验证脚本（给 CI 用）

```bash
#!/usr/bin/env bash
# 验证 helix-core 未被 IM 业务污染
# 运行：bash scripts/check-core-purity.sh

CORE_SRC="crates/helix-core/src"

IM_WORDS=(
    "temporary_id" "temporaryId"
    "im:post" "im:channel" "im:sync" "im:__bus__"
    "message_repo" "channel_event_seq" "channel_event_cursor"
    "posts/create" "sync/notify"
    "sendStatus" "unSend" "phantom"
    "post_read" "post_update" "posts_update"
    "hello\|increment_channel"
    "type1\|type2\|type3\|type6"
)

FOUND=0
for word in "${IM_WORDS[@]}"; do
    if grep -r "$word" "$CORE_SRC" --include="*.rs" -l 2>/dev/null | grep -q .; then
        echo "POLLUTION: '$word' found in helix-core"
        grep -r "$word" "$CORE_SRC" --include="*.rs" -n
        FOUND=1
    fi
done

if [ $FOUND -eq 0 ]; then
    echo "OK: helix-core is clean (no IM-specific terms found)"
else
    exit 1
fi
```

---

## 附：错误类型骨架

```rust
// helix-core/src/error.rs

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("no module accepted event: {0}")]
    NoHandler(&'static str),

    #[error("module error: {0}")]
    Module(#[from] ModuleError),

    #[error("engine not started")]
    NotStarted,
}

#[derive(Debug, Error)]
pub enum ModuleError {
    #[error("malformed inbound frame: {0}")]
    MalformedFrame(String),

    #[error("invalid command payload: {0}")]
    InvalidCommand(String),

    #[error("internal state error: {0}")]
    InternalState(String),
}

// 驱动层错误（不在 core，示意）
// pub enum DriverError { Transport(...), Storage(...), ... }
```
