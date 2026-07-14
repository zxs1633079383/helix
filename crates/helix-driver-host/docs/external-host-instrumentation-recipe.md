# 外部宿主在投影路径插录放/日志装饰器 — 权威 recipe

> 受众：把 helix 引擎内嵌作 Tauri/移动后端的**外部组装根**（如 `loopforge-tauri-im`）。
> 本文档**直接回答** loopforge W1 的 `BatchSink integration_todo`：投影面（facet②）到底该装饰什么、怎么装。
> 真源代码：`crates/helix-driver-host/src/engine.rs`（`BatchSink` + `EngineDeps`）、
> `crates/helix-driver-native/src/event_sink.rs`（`NativeEventSink`）、
> `crates/helix-driver-host/src/instrument.rs`（本文档配套的 `RecordingSink` 范例）。

---

## 0. 一句话结论（先看这个）

**投影面装饰器必须实现 host 的 [`BatchSink`]，不是只实现 core 的 [`EventSink`]。**
原因：泛型事件泵 `EngineDeps<S, H, E, C>` 与 `run_engine_loop::<S, H, E, Tr, C>` 的投影面类型参数
`E` bound 的是 **`E: BatchSink`**（host 本地 supertrait），而非 `E: EventSink`。所以
`Recording<dyn EventSink>` / 只 `impl EventSink` 的装饰器**编译期就装不进引擎**。

正解：写一个装饰器同时实现**两个** trait——`EventSink`（旁路录放 + 透传 `emit`）+ `BatchSink`
（透传 `flush`），把它当普通 sink 塞进 `EngineDeps.event_sink`。所有投影 `Effect::Emit` 都经
`EventSink::emit` 一条路出去，**100% 被装饰器截到**。loopforge 可直接复用本仓
`helix_driver_host::RecordingSink`，或抄它的形态自写。

---

## 1. 投影面在引擎里怎么流（判清关系）

```
core (helix-im)                        host pump (helix-driver-host::engine)
  ImDomainEvent --ACL-1 序列化-->        每 step:
  Effect::Emit(DomainEventBytes)  ──►   dispatch_effects(...) {
                                          for Emit(ev) => event_sink.emit(ev)   ← 逐条 EventSink::emit
                                        }
                                        event_sink.flush()                       ← step 末一次 BatchSink::flush
```

- **`EventSink`（core trait）** = 投影面的「登记意图」契约，同步非阻塞，单条
  `fn emit(&self, event: DomainEventBytes)`。**这就是投影路径**——facet② 的每条投影事件都从这里过。
  （core 不认识「批边界」运行时概念，HX-C001：core 零运行时。）
- **`BatchSink`（host supertrait）** = `pub trait BatchSink: EventSink { fn flush(&self) {} }`。
  pump 在**每次 `dispatch_effects` 之后**调一次 `flush()`，让需批 egress 的 sink（FFI vtable）攒批
  一次过边界（HX-C007）。默认 no-op（native broadcast 即时发，无需攒批）。
- **引擎 bound 的是 `BatchSink`**（`engine.rs`）：
  ```rust
  pub struct EngineDeps<S, H, E, C> { pub event_sink: Arc<E>, /* ... */ }

  pub async fn run_engine_loop<S, H, E, Tr, C>(/* ... */)
  where
      E: BatchSink + Send + Sync + 'static,   // ← 不是 EventSink，是 BatchSink
      /* ... */
  ```

两个具体 sink 都同时实现两者：
- native `NativeEventSink`：`impl EventSink`（broadcast send）+ `impl BatchSink {}`（默认 no-op）。
- ffi `FfiEventSink`：`impl EventSink`（入缓冲）+ `impl BatchSink { fn flush() { 整批编码一次过 vtable } }`。

---

## 2. `Recording<EventSink>` 够不够？——不够，必须 `Recording<BatchSink>`

| 方案 | 能否塞进 `EngineDeps.event_sink` | 为什么 |
|---|---|---|
| 只 `impl EventSink for Recording<S>` | ❌ 编译失败 | `E: BatchSink` 不满足——`Recording` 没实现 `BatchSink` |
| `Recording<S>` 同时 `impl EventSink` + `impl BatchSink`（`S: BatchSink`）| ✅ | 满足 `E: BatchSink`，像普通 sink 一样注入 |
| `Arc<dyn EventSink>` 形态的擦除装饰器 | ❌（且 `dyn` 不便）| 同样缺 `BatchSink`；且引擎要 `E: Sized` 泛型，不吃 `dyn` |

**录放完整性保证**：投影事件**唯一出口**是 `EventSink::emit`（dispatch 对每个 `Effect::Emit` 逐条调）。
装饰器在 `emit` 旁路即可截到**全部**投影；`flush` 只是批边界信号，**不携带新事件**（FFI 也只是把
已 `emit` 入缓冲的批一次过边界），所以录放放在 `emit` 旁路、`flush` 原样透传即可，零遗漏。

---

## 3. 可编译的装饰器骨架（直接抄）

本仓已落地配套范例 `helix_driver_host::RecordingSink`（`src/instrument.rs`，含单测）。核心形态：

```rust
use std::sync::Arc;
use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use helix_driver_host::BatchSink;      // host supertrait（关键）

pub struct RecordingSink<S> {
    inner: S,
    observe: Arc<dyn Fn(&DomainEventBytes) + Send + Sync>,  // 录放器钩子
}

impl<S> RecordingSink<S> {
    pub fn new(inner: S, observe: Arc<dyn Fn(&DomainEventBytes) + Send + Sync>) -> Self {
        Self { inner, observe }
    }
}

// ① 旁路录放 + 透传 emit（投影事件在此被截）
impl<S: EventSink> EventSink for RecordingSink<S> {
    fn emit(&self, event: DomainEventBytes) {
        (self.observe)(&event);   // 录放器先看到（非阻塞、不 panic）
        self.inner.emit(event);   // 再交真实 sink 出去
    }
}

// ② 透传 flush（缺这个就装不进引擎；保 HX-C007 批边界不变）
impl<S: BatchSink> BatchSink for RecordingSink<S> {
    fn flush(&self) { self.inner.flush(); }
}
```

### loopforge 组装根接法（native / Tauri 后端）

```rust
use std::sync::Arc;
use helix_driver_host::RecordingSink;                 // 本仓 re-export（add-only helper）
use helix_driver_native::event_sink::NativeEventSink; // 真实投影 sink

// 1. 建真实 native sink（broadcast egress）
let (native_sink, rx) = NativeEventSink::new();        // rx 交 event_bus loop → app.emit('im:__bus__')

// 2. 用录放器包一层（facet② 全量截获）
let recorder = my_loopforge_recorder();                // 你的录放/对账器
let sink = RecordingSink::new(
    native_sink,
    Arc::new(move |ev| recorder.record_projection(ev)),
);

// 3. 像普通 sink 一样塞进引擎（满足 E: BatchSink）
//    —— 若你直接用 host 泛型壳 engine::run_engine_loop，sink 进 EngineDeps.event_sink。
//    —— 若你用 native 的 EngineConfig 包装层，则需让 EngineConfig 接受泛型 sink（见 §4 注意）。
```

> ⚠️ **`emit`/`flush` 必须非阻塞、不 panic**：与 `EventSink::emit` 契约一致；FFI 场景 `flush` 跨
> C++ 边界回调，panic 会越界（HX-C006 生产 cdylib `panic="abort"`）。录放器内部失败应吞掉 + 计数，
> 不向上抛。

---

## 4. 注意：native `EngineConfig` 当前是「具体类型」非泛型（loopforge 需知）

`helix-driver-native::engine_loop::EngineConfig` 把 `event_sink` 字段**写死**为具体
`NativeEventSink`（`into_deps()` 直接 `EngineDeps<NativeStorage, NativeHttp, NativeEventSink, NativeClock>`）。
所以装饰器**不能**直接经现成的 native `run_engine_loop(config, ..)` 入口进去——那条路只吃裸
`NativeEventSink`。loopforge 有两条合法路径：

- **路径 A（推荐·零改 helix）**：loopforge 直接调 **host 泛型壳** `helix_driver_host::engine::run_engine_loop`
  + 自建 `EngineDeps { event_sink: Arc::new(recording_sink), .. }`，绕开 native 的 `EngineConfig` 包装层。
  native 的 `NativeStorage/NativeHttp/NativeClock/NativeTransport` 都是 pub，可自行装配进泛型 `EngineDeps`。
  这是**纯增量集成**，不需要改 helix 任何既有逻辑。

- **路径 B（需改 helix → 走 drift-review）**：把 native `EngineConfig.event_sink` 泛型化
  （`EngineConfig<E: BatchSink>`），让现成 native 入口也能吃装饰过的 sink。
  **这是对既有引擎装配 API 的改动**，本 recipe 不擅自做；若 loopforge 需要它，**须新开
  helix 分支并过 `architecture-drift-review`**（HX-C001/C002 Send 边界 + 双端回归），不在本 worktree 内改。

> 结论给 loopforge：**先走路径 A**（直连 host 泛型壳 + `RecordingSink` 包 `NativeEventSink`），
> 零侵入即可抓到 facet② 全量投影。仅当确实需要复用 native 包装入口时，才提路径 B 走 drift-review。

---

## 5. 其余三个 port 怎么装饰（一句话）

Storage / HttpRequester / FrameSender / Clock 都是 **core 的 trait**（无 host supertrait 这道坎），
loopforge 直接 `impl CoreTrait for Recording<Inner>` 透传 + 旁路即可，不需要本仓提供 helper。
投影面是**唯一**因为 `BatchSink` supertrait 而需要特别说明的 facet——本文档专门为它而写。

---

## 6. 验证

本仓 add-only helper 验证：
```bash
cargo check -p helix-driver-host          # RecordingSink 编译 + 满足 BatchSink bound
cargo test  -p helix-driver-host          # 含 recording_sink_observes_and_forwards_emit 回归锚
```
loopforge 侧端到端验证：发一条触发投影的命令 → 录放器计数 == `im:__bus__` 投影 emit 条数（参照
`crates/helix-im/docs/projection-schema.md` 的 21 个投影工厂字段集对账）。
