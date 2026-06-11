# Helix DDD 领域驱动设计 + 完整技术选型表

> **上游事实来源**
> - `00`（三栖约束底座 / 六项 core 最小能力）—— 本文 §技术选型直接锚定其裁决。
> - `01-im-as-core-contract-demands.md`（IM 第一性需求方 / 六 port / 两段式 step / correlation id 契约 / 污染红线）—— 本文 §IM 业务域 直接锚定其源码事实。
>
> **本文范围**：用 DDD 把 Helix 切成两个**互不渗透**的限界上下文（Bounded Context），给出各自的聚合 / 实体 / 值对象 / 领域事件 + ubiquitous language，画出三道防腐层（ACL），并产出每一项依赖的三栖兼容性技术选型表。
>
> **五条质量轴贯穿全文**：① 热路径 O(1) 零拷贝；② 最简够用、通用性靠真实模块挣不预建框架；③ 三栖全绿 executor-agnostic；④ 加模块不改 core / 加平台不改 core·模块；⑤ 类型驱动 make-invalid-states-unrepresentable。

---

## 1. 两个限界上下文（Bounded Context）总览

Helix 内部存在**两个语言互不相通的领域**。DDD 的核心价值就是承认它们各说各话、用 ACL 翻译，而不是让一套 ubiquitous language 横跨两边（那正是上一版 im_core 把 IM 概念渗进 core 的根因）。

```
┌──────────────────────────────────────────────────────────────────────┐
│  BC-A  Platform Core Domain  (helix-core)                              │
│  "运行确定性业务模块的操作系统"——不认识任何业务                          │
│  语言: Module / Port / Effect / Tick / ExecutionShell / Correlation    │
│  聚合: ModuleHost · ExecutionShell · PortRegistry                       │
└──────────────────────────────────────────────────────────────────────┘
                        ▲  只通过 trait 契约 + Effect/Tick 数据对话
                        │  (没有任何 IM 类型穿过这条线)
                        ▼
┌──────────────────────────────────────────────────────────────────────┐
│  BC-B  IM Business Domain  (helix-im)                                  │
│  "会话即时通讯的确定性内核"——不认识 tokio/sqlx/web-sys                   │
│  语言: Channel / Message / Cursor / PendingSend / Reconciliation       │
│  聚合: Channel(消息+游标) · PendingSend · SyncSession                    │
└──────────────────────────────────────────────────────────────────────┘
```

**关键不渗透原则（DDD context map = "Customer/Supplier" + 强 ACL）**：
- BC-B（IM）是 BC-A（core）的 **customer**；BC-A 是 supplier。
- 但 supplier **完全不知道** customer 是谁：core 的 ubiquitous language 里没有 "channel / message / cursor / temporaryId / 对账"。
- 所有 IM 词汇穿越边界时**降级为 core 的通用词汇 + 不透明字节/参数**（见 §4 ACL）。

> 判据回指 `01 §4 / §8`：删掉 helix-im，helix-core 必须仍能编译且 grep 不到任何 IM 私有词。这就是"两个 BC 不渗透"的可执行验收。

---

## 2. BC-A：Platform Core Domain（`helix-core`）领域模型

### 2.1 Ubiquitous Language（核心层通用词典）

| 术语 | 定义 | **绝不包含** |
|---|---|---|
| **Module（模块）** | 一个确定性业务单元。实现 `Module` 契约即可被宿主装载。core 只知道它有名字、能 `accepts` 某帧、能 `process` 产出 effect。 | core 不知道某个 Module 是 IM 还是 presence |
| **Port（端口）** | core 与平台之间的契约 trait：Transport / Storage / Clock / EventSink / Id / Random / HttpRequester。 | 不含任何具体 crate（tokio/sqlx/web-sys） |
| **Tick（节拍 / 入站事件）** | 喂给执行壳的一个确定性输入：入站帧 / 端口响应（带 correlation）/ 计时触发 / 应用命令 / now_ms。 | 不含 IM 的 14 类 type（那是 IM 解析后的事） |
| **Effect（副作用意图）** | 模块吐出的"想做的 I/O"声明：Send / Persist / Emit / HttpRequest / ScheduleTimer / CancelTimer，**可携带 Correlation**。core 不执行，driver 执行。 | 不含 IM 填进 variant 的内容 |
| **Correlation（关联令牌）** | 把一个 Effect 与它未来的端口响应配对的不透明令牌。**这是 `01 §6` 对 core 的硬性增量需求**（peek/commit 写成功才推）。 | core 不知道它锚的是 cursor 还是别的 |
| **ExecutionShell（确定性执行壳）** | 纯函数 `step(Tick) -> Vec<Effect>`：同序列入 → 同序列出，与平台/时间/线程无关。 | 不做 I/O、不持运行时、不读时钟 |
| **PortRegistry / 注入** | host 把具体 adapter（实现 Port trait）注入给执行壳。 | — |

### 2.2 聚合（Aggregate）与归属

> DDD 聚合 = 一致性边界 + 唯一聚合根。core 域里聚合都**很瘦**（core 是机制不是业务），这正符合质量轴②"最简够用"。

#### 聚合 A1：`ExecutionShell`（聚合根，确定性边界的守门人）

```rust
// helix-core/src/engine.rs
/// 确定性执行壳。聚合根：唯一允许驱动模块状态推进的入口。
/// 一致性不变量：step 内不发生任何 I/O / 不读时钟 / 不 spawn。
pub struct ExecutionShell {
    host: ModuleHost,            // 子实体：模块宿主
    scratch: EffectBuffer,       // 值对象：单 step 内复用的 effect 收集器(零分配复用)
}

impl ExecutionShell {
    /// 纯函数语义：相同 tick 序列 + 相同模块初态 ⇒ 相同 effect 序列。
    pub fn step(&mut self, tick: Tick<'_>) -> Result<EffectBatch<'_>, CoreError> {
        self.scratch.clear();                       // O(1) 复用，不重新分配
        let frame = InboundFrame::from_tick(tick)?; // 边界校验：非法 tick fail-fast
        self.host.dispatch(frame, &mut EffectSink::new(&mut self.scratch))?;
        Ok(self.scratch.batch())                    // 借出视图，driver 同步 drain
    }
}
```

- **不变量**：`step` 是纯的（除模块自身状态转移外）。任何 `await` / `tokio::spawn` / `Instant::now` 出现在 step 调用链 = 破坏聚合不变量。
- **质量轴①**：`EffectBuffer` 跨 step 复用（`clear` 而非重建），`EffectBatch` 是借出视图（`&'_`），driver 同步 `drain` 后即归还 → 零额外分配、零拷贝。

#### 聚合 A2：`ModuleHost`（聚合根，模块注册/路由/生命周期）

```rust
// helix-core/src/module_host.rs
pub struct ModuleHost {
    modules: Vec<Box<dyn Module>>,   // 子实体集合，按注册序
}
/// Module 契约：业务模块实现它即可被装载。core 不 import 任何模块 crate。
pub trait Module: MaybeSend + 'static {
    fn name(&self) -> ModuleName;                         // 值对象
    fn accepts(&self, frame: &InboundFrame) -> bool;     // 路由判定（纯）
    fn process(&mut self, frame: InboundFrame, fx: &mut EffectSink) -> Result<(), CoreError>;
    fn start(&mut self, fx: &mut EffectSink) -> Result<(), CoreError> { Ok(()) }
    fn stop(&mut self, fx: &mut EffectSink) -> Result<(), CoreError> { Ok(()) }
}
```

- **质量轴④**：加新业务模块 = `register(impl Module)`，**不改 ModuleHost / ExecutionShell 任何一行**（开闭原则）。
- **不变量**：路由是确定的——`accepts` 必须纯（无 I/O），首个 accept 的模块独占该帧（与 `01 §3` IM dispatcher 单点喂帧一致）。

#### 聚合 A3：`PortRegistry`（聚合根，端口契约集合 + 注入边界）

```rust
// helix-core/src/ports.rs —— 仅 trait，无实现
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait Transport: MaybeSend + MaybeSync + 'static {
    type Error: PortError;
    async fn send(&mut self, frame: OutboundFrame) -> Result<(), Self::Error>;
    async fn recv(&mut self) -> Result<InboundBytes, Self::Error>;
}
// Storage / HttpRequester 同款 cfg-gated async-trait（见 01 §1.2/§1.6）
pub trait Clock: Clone + MaybeSend + 'static { fn now_ms(&self) -> u64; }
pub trait EventSink: MaybeSend + MaybeSync + 'static { fn emit(&self, ev: DomainEventBytes); }
pub trait Id:    Clone + MaybeSend + 'static { fn new_id(&self) -> RawId; }
pub trait Random:Clone + MaybeSend + 'static { fn next_u64(&self) -> u64; }
```

> **重要**：core 的 `EventSink::emit` 接受的是 `DomainEventBytes`（已序列化的不透明事件），**不是** IM 的 `DomainEvent` enum——后者属于 BC-B（见 §3.4）。这是不渗透的关键：core 不持有 IM 的事件类型。

### 2.3 值对象（Value Object，全部不可变、make-invalid-states-unrepresentable）

| 值对象 | 形态 | 设计要点（质量轴⑤） |
|---|---|---|
| `Tick<'a>` | `enum { Inbound(InboundBytes<'a>), PortReply(Correlation, PortOutcome), Timer(TimerId), Command(AppCommand<'a>) } + now_ms` | tagged union；非法组合不可表达（端口响应必带 Correlation） |
| `Effect<'a>` | `enum { Send{..}, Persist{corr: Correlation, ..}, Emit(DomainEventBytes), HttpRequest{corr, ..}, ScheduleTimer{id, after_ms}, CancelTimer(TimerId) }` | Persist/HttpRequest 强制带 Correlation（类型层逼模块走 peek/commit） |
| `Correlation` | `struct(u64)` newtype | 不透明令牌；core 不解释其语义 |
| `PortOutcome` | `enum { Ok(ReplyBytes), Err(PortErrCode) }` | 端口成功/失败二态；`01 §6` 的 `PersistResult{ok}` 即此 |
| `ModuleName` | `&'static str` newtype | 零分配静态串 |
| `TimerId` | `struct(u64)` newtype | — |
| `InboundBytes<'a>` / `OutboundFrame` / `DomainEventBytes` | `bytes::Bytes` 包装 | 零拷贝搬运，core 不解析内容 |

### 2.4 领域事件（core 视角）

core 域**自身几乎不产生领域事件**——它是机制层。唯一的 core 级事件是 `ModuleLifecycle`（Started/Stopped），用于宿主可观测性。**业务领域事件（PostReceived 等）属于 BC-B**，core 只把它们当 `DomainEventBytes` 搬运。

---

## 3. BC-B：IM Business Domain（`helix-im`）领域模型

> 全部锚定 `01` 的 FRESH_IM_SOURCE 源码事实。IM 内核是 **sans-IO 纯状态机**：不 import tokio/sqlx/web-sys，只跟 `helix-core` 的 trait + Effect/Tick 数据对话。

### 3.1 Ubiquitous Language（IM 业务词典）

| 术语 | 定义（源码锚点） | 对应 core 的什么 |
|---|---|---|
| **Channel（会话）** | 一个频道的消息集合 + 单调 event cursor。聚合根。 | core 不知道有 Channel |
| **Message（消息）** | 一条消息。PK 语义特殊：`temporaryId`（UI 注入）→ server `id` **值级覆盖**（`ON CONFLICT id=excluded.id`）。 | core 看到的是 `Persist{conflict_key}` 的一行 |
| **Cursor（游标）** | per-channel `last_event_seq`，单调推进（MAX guard），**写成功才推**。 | core 的 `cursor_upsert(scope_key,value)` 单调原语 |
| **PendingSend（待确认发送）** | 已乐观落库、等 echo/sync 对账的发送；15s 超时 → `unSend`（无自动补发）。 | core 的 `ScheduleTimer/CancelTimer` + Correlation |
| **Reconciliation（对账）** | echo 与 sync **同一对账点**：按 temporaryId 值级覆盖回 `sent` + 撤超时。 | core 看不到对账，只看到 Persist + CancelTimer + Emit |
| **SyncSession（同步会话）** | 一次 `/sync/notify(from_seq)` 拉取：响应 4 kind（NoChange/Events/Snapshot/TooLong）+ ≥500 续拉。 | core 看到 HttpRequest + PortReply |
| **Gate（缺口闸门）** | `seq>cursor+1` 时 buffer + 1s timer，补齐则 cancel。 | core 的 ScheduleTimer/CancelTimer |
| **DomainEvent** | 8 族强类型 enum（PostReceived/Updated/Deleted/BatchUpdated/Read/ChannelReadEcho/ChannelUpdateByPost/SyncTooLong）。 | core 收 `DomainEventBytes`（序列化后） |

### 3.2 聚合 B1：`Channel`（聚合根 —— 消息 + 游标的一致性边界）

> **DDD 关键裁决**：`Message` 不是独立聚合根，而是 `Channel` 聚合内的实体；`Cursor` 是 `Channel` 的值对象。理由：cursor 推进与消息落库是**同一一致性边界**（`01 §6` peek/commit：写消息成功才推 cursor，二者必须事务式同步）。把它们拆成两个聚合会破坏"写成功才推"不变量。

```rust
// helix-im/src/channel.rs  —— 纯状态机，零 I/O
pub struct Channel {
    id: ChannelId,                       // 值对象（聚合根标识）
    cursor: Cursor,                      // 值对象：last_event_seq + MAX guard 语义
    gate: Option<Gate>,                  // 实体：缺口闸门（buffer + pending timer）
    pending_commits: PendingCommitMap,   // 实体：corr -> target_seq（写成功才推）
    buffered: BTreeMap<Seq, EventEnvelope>, // 乱序缓冲（contiguous flush 用）
}

impl Channel {
    /// 入站 event（echo / sync），纯逻辑。产出 effect，不做 I/O。
    /// 回指 01 §2.1 gate 判定 + 双 flush + 去重。
    pub fn ingest(&mut self, ev: EventEnvelope, fx: &mut EffectSink, now_ms: u64)
        -> Result<(), ImError>
    {
        match ev.seq.cmp(&(self.cursor.value() + 1)) {
            Ordering::Equal   => self.apply_and_flush_contiguous(ev, fx, now_ms),
            Ordering::Less    => Ok(()),                  // dup drop（幂等去重）
            Ordering::Greater => self.buffer_and_arm_gate(ev, fx),  // ScheduleTimer 1s
        }
    }
}
```

**Channel 聚合不变量**（全部来自 `01`）：
1. `cursor` 单调不减（MAX guard）。
2. cursor 只在收到 `PortReply(corr, Ok)`（持久化成功）后推进——**写成功才推**（`01 §6` 两段式）。
3. `seq <= cursor` 的入站事件一律 drop（幂等去重）。
4. `flush_contiguous`（WS 严格 +1 遇空洞停）vs `flush_up_to`（sync 跳空洞到权威水位）落在 peek 阶段算 `target_seq` 的纯规则上。

### 3.3 聚合 B2：`PendingSend`（聚合根 —— 发送对账的一致性边界）

> 独立聚合，因为它的生命周期（乐观落库 → 等对账 → 15s 超时 / 对账成功）与 Channel 的 cursor 推进**正交**：一条消息的对账状态不影响 channel 游标。

```rust
// helix-im/src/pending_send.rs
pub struct PendingSend {
    temporary_id: TemporaryId,   // 值对象：聚合根标识(UI 注入，01 §5)
    status: SendStatus,          // 值对象：Local | Sending | Sent | UnSend
    timeout: TimerId,            // 关联的 15s 超时令牌
}
pub enum SendStatus { Local, Sending, Sent, UnSend }   // make-invalid-states-unrepresentable

impl PendingSend {
    /// echo/sync 到达 = 对账（01 §5）。echo 与 sync 走完全相同 effect 序列。
    pub fn reconcile(&mut self, server_id: ServerId, fx: &mut EffectSink) {
        // 1) Persist: ON CONFLICT(temporary_id) 值级覆盖回 sent
        // 2) CancelTimer(self.timeout)   —— 撤 15s
        // 3) Emit(PostReceived)
        self.status = SendStatus::Sent;
    }
    /// 15s TimerFired → unSend，不产出任何重试 effect（无自动补发，01 §2.1/§5）
    pub fn on_timeout(&mut self, fx: &mut EffectSink) {
        if self.status != SendStatus::Sent { self.status = SendStatus::UnSend; /* Persist update */ }
    }
}
```

**C006 在状态机里的体现**（`01 §5`）：echo 先到 → `CancelTimer` 让 15s 永不 fire；timer 先到 → 标 `UnSend`，迟到 echo 仍按 temporaryId 覆盖回 `Sent`。两种到达次序收敛到一致终态——这是纯状态机的确定性保证，不依赖时序。

### 3.4 聚合 B3：`SyncSession`（聚合根 —— 一次同步拉取的边界）

```rust
// helix-im/src/sync_session.rs
pub struct SyncSession { from_seq: Seq, corr: Correlation, phase: SyncPhase }
pub enum SyncResponse {            // 入站端口响应（01 §3.1 C）
    NoChange,
    Events(Vec<EventEnvelope>),    // ≥500 → needs_continuation
    Snapshot(ChannelSnapshot),
    TooLong { reset_to: Seq },     // cursor = reset_to - 1 + Emit(SyncTooLong)
}
```

分桶顺序铁律（`01 §3.2 B`）：`type1 upsert → type2 edit → type3 revoke → type6 read` 固定序产出 Persist effect。

### 3.5 值对象 & 领域事件

| IM 值对象 | 形态 |
|---|---|
| `ChannelId` / `ServerId` / `TemporaryId` / `Seq` | newtype，零混淆（类型层禁止 `Seq` 当 `ChannelId` 用） |
| `SendStatus` | 4 态 enum |
| `Cursor` | `struct{ value: Seq }`，仅 `try_advance(MAX guard)` |
| `EventEnvelope` | 解析后的强类型入站事件（IM 解析 14 类 WS type 的产物） |

**IM 领域事件（8 族 enum，`01 §3.2 A`）**：
```rust
pub enum DomainEvent {
    PostReceived(..), PostUpdated(..), PostDeleted(..), PostBatchUpdated(..),
    PostRead(..), ChannelReadEcho(..), ChannelUpdateByPost(..), SyncTooLong(..),
}
```
→ 在 **ACL 边界**序列化为 `im:post:received` 等 wire 名 + 包成 `DomainEventBytes` 交给 core EventSink（见 §4.1）。core 永远看不到这个 enum。

---

## 4. 防腐层（Anti-Corruption Layer）—— 三道翻译墙

DDD 的 ACL = 在边界放一层翻译器，**让一边的模型变化不污染另一边**。Helix 有三道 ACL。

### 4.1 ACL-1：IM 业务域 ⇆ core 平台域（最关键，防 IM 渗进 core）

> 位置：`helix-im` crate 内部的 `acl/` 模块（**翻译器属于 customer 一侧**，这是 DDD 标准做法——supplier 不该为 customer 写翻译）。

| IM 域概念（BC-B） | ──ACL 翻译──▶ | core 域概念（BC-A） |
|---|---|---|
| `DomainEvent::PostReceived` | 序列化为 `im:post:received` JSON + `Bytes` 包装 | `Effect::Emit(DomainEventBytes)` |
| `Channel.cursor.try_advance(seq)` | `scope_key = channel_id.to_string()` | `Effect::Persist{ cursor_upsert(scope_key, value), corr }` |
| `Message{temporary_id, 23 字段}` | `conflict_key = "temporary_id"` + row 列表 | `Effect::Persist{ batch_upsert(table, rows, conflict_key), corr }` |
| `/posts/create` 直发 / `/sync/notify?from_seq=` | 拼 URL + body | `Effect::HttpRequest{ req, corr }` |
| WS 出站 `im_send_message(action,data,seq)` | 序列化帧 | `Effect::Send(OutboundFrame)` |
| 入站不透明 `InboundBytes` | 解析 14 类 WS type | `EventEnvelope`（IM 强类型，反向翻译） |
| `pending_commits[corr]` | corr 不透明令牌 | `Correlation`（core 只透传） |

**ACL-1 的不渗透保证**：
- core 的 trait 签名里**只有** `table: &str` / `conflict_key: &str` / `scope_key: &str` / `Bytes` / `Correlation` / `after_ms: u64` —— 全是通用参数（`01 §4` 红线表）。
- `temporary_id`、`channel_event_seq`、`im:` 前缀、表名、23 字段、15000ms —— **全在 ACL-1 这一侧拼**，作为参数值穿过去，不进类型。
- **验收**：删掉 `helix-im`（含其 `acl/`），`helix-core` 编译通过 + grep 不到任一红线词。

### 4.2 ACL-2：core 平台域 ⇆ 平台驱动层（防具体 crate 渗进 core）

> 位置：每个 `helix-driver-*` crate 内（**adapter 属于 driver 一侧**）。这是六边形架构的 ports & adapters：core 定义 port trait，driver 实现并把具体 crate 类型挡在外面。

| core port（trait） | native adapter | wasm adapter | ffi adapter |
|---|---|---|---|
| `Transport` | `tokio-tungstenite` WebSocketStream | `ws_stream_wasm` / web-sys WebSocket | C ABI 回调 + futures-channel |
| `Storage` | `sqlx::SqlitePool` (WAL) | `sqlite-wasm-rs` + OPFS | host SQLite handle via C ABI |
| `Clock` | `SystemTime` | `js_sys::Date::now()` | host `fn()->u64` |
| `HttpRequester` | `reqwest` | `gloo-net`/fetch | host http 回调 |
| `EventSink` | `tauri AppHandle.emit` | `wasm-bindgen` → JS callback | C ABI `on_event_fn` |
| `Id`/`Random` | `uuid`/`rand` | 同 + `getrandom js` | host 注入 |

**ACL-2 的不渗透保证**：core 的 `Cargo.toml` **不含** tokio/sqlx/reqwest/tungstenite/web-sys（`01 §00` / `00 §4.2`）；这些只出现在 `[target.'cfg(...)'.dependencies]` 的 driver crate 里。

### 4.3 ACL-3：FFI / WASM 宿主边界（防外部表示渗进域模型）

> 位置：`helix-driver-ffi`（uniffi/cbindgen）与 `helix-driver-wasm`（wasm-bindgen）的导出层。把 C ABI / JsValue 这类外部表示翻译成 core 的 `Tick`，把 `Effect` 翻译成外部调用。

- FFI：C 侧 push 的 `*const u8` **必须在回调返回前拷贝到 Rust 堆**（`01 §1.1` 所有权红线），再包成 `Tick::Inbound`。
- WASM：`JsValue` 是 `!Send`，只能在 `spawn_local` 里处理；ACL-3 在此把 JS 事件转成 `Tick`，绝不让 `JsValue` 漏进 `helix-core`/`helix-im`（二者对 wasm-bindgen 零依赖）。

---

## 5. 完整技术选型表（每项：选什么 / 为什么 / 备选 / 三栖兼容 native·ffi·wasm）

> 兼容性以 `wasm32-unknown-unknown` 为最严格基准（`00 §1`）。✅=可用 / ⚠️=需条件 / ❌=不可用且必须挡在驱动层。

### 5.1 async 抽象

| 维度 | 选型 | 为什么 | 备选 | native | ffi | wasm |
|---|---|---|---|---|---|---|
| port trait async | `async-trait` cfg-gated：`(?Send)` on wasm | dyn-safe；wasm 单线程无 Send（`00 §3.1`） | AFIT+`trait_variant`（不 dyn-safe，留给静态路径） | ✅ | ✅ | ✅ |
| core 内部静态路径 | AFIT / `impl Future`（零装箱） | 热路径零分配（轴①） | async-trait（有 Box 开销） | ✅ | ✅ | ✅ |
| Send/!Send 桥 | 自写 `MaybeSend/MaybeSync`（30 行 cfg） | 自持可控，不引第三方（轴②） | `tokio-with-wasm`（改并发语义，维护风险，`00 §3.4` 已否决） | ✅ | ✅ | ✅ |
| channel | `futures-channel`（mpsc/oneshot） | 无 executor 依赖，wasm 可编（`00 §3.3`） | `tokio::sync`（reactor 依赖 ❌ wasm） | ✅ | ✅ | ✅ |
| spawn | core `platform::spawn` cfg-gated | native→`tokio::spawn` / wasm→`spawn_local` | — | ✅ | ✅ | ✅ |

> **轴③ 命根子**：core 绝不 bake executor（`01 §2.2`）。模块只吐 timer/effect，driver 在自己的 executor 兑现。

### 5.2 序列化

| 用途 | 选型 | 为什么 | 备选 | native | ffi | wasm |
|---|---|---|---|---|---|---|
| DomainEvent → wire（ACL-1） | `serde_json` | IM wire 是 JSON（Mattermost 系），与现有契约一致 | `rmp-serde`(MessagePack，体积优但破契约) | ✅ | ✅ | ✅ |
| 跨 FFI 结构 | `postcard`（no_std, 紧凑二进制） | 零拷贝友好、wasm/ffi 体积小（轴①⑤） | bincode（无 no_std）、protobuf（重） | ✅ | ✅ | ✅ |
| WASM ⇆ JS 互操作 | `serde-wasm-bindgen` | 直接 JsValue⇆Rust，避免 JSON 中转双解析 | `JSON.stringify`+serde_json(双解析慢) | n/a | n/a | ✅ |
| 内部 effect/tick | **不序列化**（进程内传 Rust 值） | 进程内零序列化开销（轴①） | — | ✅ | ✅ | ✅ |

> 裁决：core/IM 内部传 **Rust 值**（不序列化）；只在三道 ACL 边界才序列化。`serde`+`serde_json` 进 `helix-core` 允许（`00 §4.1` 无 reactor 依赖）；`serde-wasm-bindgen` 只进 `helix-driver-wasm`。

### 5.3 错误处理（分层）

| 层 | 选型 | 为什么 | native | ffi | wasm |
|---|---|---|---|---|---|
| `helix-core` port/engine | `thiserror` 强类型 enum（`CoreError`、`PortError`） | 库层必须强类型可匹配（轴⑤），无 anyhow 抹平 | ✅ | ✅ | ✅ |
| `helix-im` 域错误 | `thiserror`（`ImError`）+ `From<CoreError>` | 域错误强类型；归一到 core 错误链 | ✅ | ✅ | ✅ |
| driver 适配层 | `thiserror` 包裹具体 crate err 实现 `PortError` | 把 sqlx/reqwest err 翻译成 port 错误（ACL-2） | ✅ | ✅ | ✅ |
| FFI/WASM 边界 | 转 `Result<T, String>` / JsError / uniffi error enum | 外部 ABI 不能传 Rust enum | ✅ | ✅ | ✅ |

> **anyhow 仅允许**出现在 driver 的 `main`/bootstrap（应用顶层），**不进** `helix-core`/`helix-im`（库层禁 anyhow，轴⑤）。

### 5.4 存储（Storage port 三端实现）

| 端 | 选型 | 为什么 | 备选 | 可用性 |
|---|---|---|---|---|
| native | `sqlx::SqlitePool` + WAL，`default-features=false, features=["sqlite","runtime-tokio"]` | 编译期校验 + 池化 + 热路径 batch_upsert O(1)（轴①） | `rusqlite`+spawn_blocking | ✅ native |
| ffi(mobile) | host SQLite handle via C ABI（`query_fn/execute_fn`） | 复用宿主已有 DB，零额外引擎 | uniffi 暴露 rusqlite | ✅ ffi |
| wasm(browser) | `sqlite-wasm-rs` + OPFS `opfs-sahpool` VFS | 唯一能在浏览器持久化 SQLite 的路径 | IndexedDB(无 SQL，需另写 adapter) | ⚠️ 须在 Web Worker 初始化(`00 §2.2`) |
| **禁入 core** | `sqlx` / `rusqlite` 进 driver | sqlx 依赖 tokio+mio+getrandom ❌ wasm；rusqlite 依赖 pthread ❌ wasm（`00 §4.2`） | — | ❌ core |

### 5.5 传输（Transport port 三端实现）

| 端 | 选型 | 为什么 | 备选 | 可用性 |
|---|---|---|---|---|
| native | `tokio-tungstenite`（native-tls） | 成熟 WS + tokio reactor | `tungstenite`(同步) | ✅ native |
| wasm | `ws_stream_wasm`（直接 Sink+Stream） | 最干净 wasm WS；省手桥 mpsc | `web-sys WebSocket`+手桥 futures mpsc | ⚠️ WsMessage→Bytes 手转(`00 §2.1`) |
| ffi | C ABI 回调 + `futures-channel` 包装 | host 持有 socket，Rust 收回调 | uniffi callback interface | ✅ ffi（buffer 回调前拷贝） |
| **禁入 core** | tokio-tungstenite 进 driver `[target.'cfg(not(wasm32))']` | 依赖 mio ❌ wasm（`00 §4.3`） | — | ❌ core |

### 5.6 ID / 时间注入

| 维度 | 选型 | 为什么 | native | ffi | wasm |
|---|---|---|---|---|---|
| Clock | port `now_ms()` 注入；测试用 `MockClock{AtomicU64}` | `Instant`/`SystemTime` ❌ wasm32-unknown（`00 §2.3`）；确定性回放 | `SystemTime` | host `fn()->u64` | `js_sys::Date::now()` |
| Id（core port） | `uuid::Uuid::new_v4()` | 标准 UUID | rand-based | host 注入 | uuid + `getrandom js` |
| getrandom | **workspace 级** `getrandom={features=["js"]}` | 任何传递依赖 getrandom 的路径 wasm link 失败（`00 §2.2`） | OS entropy | — | ⚠️ 必须 js feature |
| **IM 主路径** | temporaryId 由 **UI 注入**，内核不生成（`01 §5`） | sans-IO 确定性回放（轴③⑤） | — | — | — |

### 5.7 FFI 绑定（uniffi vs cbindgen）

| 维度 | 选型 | 为什么 | 备选 | 适用端 |
|---|---|---|---|---|
| 移动端（Swift/Kotlin） | **uniffi** | 自动生成 Swift/Kotlin binding + 类型安全 callback interface + error enum，省手写 C 头 | cbindgen（仅 C 头，手写桥接量大） | ffi |
| 纯 C / C++ 宿主 | **cbindgen**（`extern "C"` + `repr(C)` + opaque ptr） | C/C++ 直接 include，无 uniffi runtime 开销 | uniffi（带 runtime） | ffi |
| 裁决 | helix-driver-ffi 默认 uniffi（移动优先），保留 cbindgen 导出 opaque handle | 移动是 ffi 主战场；C 宿主走 opaque ptr | — | ffi |
| 兼容性 | uniffi/cbindgen 都 **仅 native 编译**（`crate-type=["cdylib","staticlib"]`） | wasm 不走 FFI 走 wasm-bindgen | — | ❌ wasm/❌ 不进 core |

### 5.8 WASM 绑定

| 维度 | 选型 | 为什么 | native | ffi | wasm |
|---|---|---|---|---|---|
| JS 互操作 | `wasm-bindgen` + `wasm-bindgen-futures` | 标准 wasm⇆JS；`spawn_local` 执行器 | — | — | ✅ |
| Web API | `web-sys`（WebSocket/MessageEvent/...）+ `js-sys` | 浏览器 WS/时钟/OPFS | — | — | ✅ |
| 定时器 | `gloo-timers`（futures feature） | wasm 无 tokio::time | — | — | ✅ |
| 打包 | `wasm-pack` | 标准 wasm 产物 + wasm-opt 集成 | — | — | ✅ |
| 裁决 | 全部**仅进** `helix-driver-wasm` `[target.'cfg(wasm32)']` | native 编译会失败（`00 §4.2`） | ❌ | ❌ | ✅ |

### 5.9 测试（确定性回放是验收命根子）

| 维度 | 选型 | 为什么 | native | ffi | wasm |
|---|---|---|---|---|---|
| 单元/确定性回放 | 标准 `#[test]` + `MockClock`+`MockPorts` 喂 Tick 序列断言 Effect 序列 | sans-IO 核心验收：同序列入→同序列出（轴③） | ✅ | ✅ | n/a |
| 属性测试 | `proptest`：随机 Tick 序列断言不变量（cursor 单调 / C006 收敛 / 去重幂等） | 覆盖 echo↔timer 两种到达次序收敛（`01 §5`） | ✅ | ✅ | ⚠️(慢，CI 跑 native) |
| wasm 端到端 | `wasm-bindgen-test`（`wasm-pack test --headless`） | 验证 wasm adapter（OPFS/web-sys WS）真跑通 | — | — | ✅ |
| ffi 端到端 | uniffi 生成 binding + Swift/Kotlin host 测试 | 验证 C ABI 所有权拷贝正确 | — | ✅ | — |
| 裁决 | 域逻辑测试**只跑 native**（快+确定）；adapter 测试各端各跑 | 域是纯的，三端行为应一致 → native 测够；adapter 才需各端 | — | — | — |

> **回放契约测试**（IM 验收核心）：构造 `[send(tid), Timer(15s), echo(tid,server_id)]` 与 `[send(tid), echo(...), Timer(15s)]` 两序列，断言**终态一致**（C006，`01 §5`）；构造 `[Events, PersistResult(ok)]` 断言 cursor 推进、`[Events, PersistResult(err)]` 断言 cursor 不动（`01 §6` 写成功才推）。

### 5.10 WASM 体积优化（`00 §5`）

| 手段 | 选型 | 为什么 | wasm |
|---|---|---|---|
| profile | `opt-level="z"` + `lto=true` + `codegen-units=1` + `panic="abort"` + `strip=true` | 最小体积；abort 去 unwind 表省 ~20% | ✅ |
| post-process | `wasm-opt -Oz` | 额外减 10–30% | ✅ |
| 分析 | `twiggy top/dominators` | 定位最大主导子树（常是 serde/panic machinery） | ✅ |
| feature 门控 | `#[cfg(feature="im")]` 门控整模块 | 不用的业务不编译（轴②④） | ✅ |
| allocator | 默认 `dlmalloc`（不用 wee_alloc 已无维护） | 安全优先 | ✅ |

---

## 6. 两个 BC 如何"不互相渗透"——可执行检查清单

| 检查项 | 命令 / 判据 | 违反即污染 |
|---|---|---|
| core 不识 IM 词 | 删 `helix-im` 后 `cargo build -p helix-core` 通过 + `rg -i 'temporary_id\|channel_event\|im:post\|/posts/create\|mattermost' crates/helix-core` 空 | `01 §4` 红线 |
| core 不含运行时 crate | `rg 'tokio\|sqlx\|reqwest\|tungstenite\|web-sys' crates/helix-core/Cargo.toml` 空 | `00 §4.2` |
| IM 不含运行时 crate | 同上对 `helix-im/Cargo.toml` 空 | sans-IO 纯逻辑 |
| core 三栖编译 | `cargo build -p helix-core --target wasm32-unknown-unknown` 绿 | `00 §1` 最严基准 |
| 域逻辑确定性 | proptest：任意 Tick 序列 cursor 单调 + C006 收敛 | `01 §5/§6` |
| 加模块不改 core | 新增 `helix-presence` 只 `register()`，core/engine diff 为空 | 轴④ 开闭 |

---

## 7. crate 依赖方向（DDD 依赖倒置的物化）

```
helix-driver-native ─┐
helix-driver-wasm   ─┼─▶ helix-im ─▶ helix-core
helix-driver-ffi    ─┘                  ▲
        └─────────────────(实现 ports)──┘
```

- 依赖箭头**只能向左/向上**：driver → im → core；driver 实现 core 的 port trait（依赖倒置：高层 core 定义接口，低层 driver 实现）。
- **禁止反向**：core 不依赖 im / driver；im 不依赖 driver。
- ACL-1 在 `helix-im/acl/`；ACL-2 在各 `helix-driver-*`；ACL-3 在 ffi/wasm driver 的导出层。

---

## 8. 结论

1. **两个限界上下文**：core 平台域（Module/Port/Effect/Tick/ExecutionShell/Correlation）与 IM 业务域（Channel/Message/Cursor/PendingSend/Reconciliation/SyncSession）语言互不相通，靠三道 ACL 翻译，**不共享 ubiquitous language**。
2. **聚合归属**：core 聚合瘦（ExecutionShell/ModuleHost/PortRegistry，纯机制）；IM 聚合按一致性边界切——`Channel`（消息+游标，因"写成功才推"必须同边界）、`PendingSend`（对账，与游标正交）、`SyncSession`（一次拉取）。
3. **三道防腐层**：ACL-1（IM⇆core，在 customer 侧 `helix-im/acl/`，把 IM 词全降级为通用参数）；ACL-2（core⇆driver，六边形 ports&adapters，把 tokio/sqlx/web-sys 挡在 driver）；ACL-3（FFI/WASM 宿主边界，把 C ABI/JsValue 翻译成 Tick，所有权拷贝红线）。
4. **技术选型**：全部锚定 wasm32-unknown-unknown 最严基准 + executor-agnostic（async-trait cfg-gated + futures-channel + MaybeSend）+ 库层 thiserror（禁 anyhow）+ 三端存储/传输各自 adapter + 移动 uniffi/纯 C cbindgen + 确定性回放测试（MockClock + proptest C006 收敛）。
5. **五条质量轴落地**：① 热路径 EffectBuffer 复用零拷贝 + batch_upsert O(1)；② 聚合瘦、不预建插件框架、MaybeSend 自写 30 行；③ 三栖全绿 + 域逻辑只跑 native 测；④ 加模块=register、加平台=实现 port，core 零改；⑤ tagged union Tick/Effect/SendStatus + newtype 防混淆 + 库层强类型错误。

> **教训沉淀（再次锚定 `00`/`01`）**：YAGNI 边界在三栖目标集合确定后才画——Transport/Storage/Clock 是命根子不可砍；IM 知识以源码为准；getrandom js feature 必须 workspace 级；core 删 IM 后必须仍编译且 grep 不到红线词，这是两个 BC 不渗透的最终可执行验收。
