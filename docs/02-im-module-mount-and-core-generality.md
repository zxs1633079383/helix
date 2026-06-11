# IM 模块如何挂载到 helix-core + 实时核通用性检验

> **唯一事实来源**：FRESH_IM_SOURCE（2026-06-08 IM 源码实测）+ `01-im-as-core-contract-demands.md`（core 契约需求清单）。
> 本文不使用训练记忆（cses-client message-v3 旧知识已作废）。
>
> **本文回答 5 件事**
> 1. IM 模块如何实现 core 的 `Module` trait（init/handle/tick/drop），IM 状态机如何活在 sans-IO 模型，4-worker hash 分片 + 1s gate timer 怎么表达。
> 2. 挂载流程完整时序（host 启动 → 注册 IM → core 接 port → 跑起来）。
> 3. 用 transport / recovery·cursor / 背压 / 心跳重连 四把尺检验 core ports 够不够、缺什么多什么。
> 4. 第二模块（presence）设想，证明 core 不与 IM 绑定。
> 5. C006 / temporaryId / "写成功才推 cursor" 在该模型下的落点。
>
> 命名：`helix-core`（核心层）/ `helix-im`（IM 模块）/ `helix-driver-{native,wasm,ffi}`（平台驱动）。
>
> 全文严格区分 **【core 通用必备】** vs **【IM 专属·不能进 core】**。

---

## 0. 先钉死执行壳形状（后面全部基于它）

来自 `01` 文档 §3/§6 的结论，core 执行壳是一个**两段式、带 correlation id 的纯函数循环**：

```rust
// 【core 通用】helix-core/src/engine.rs —— 与任何业务无关
pub enum Tick {
    Inbound(InboundFrame),          // driver 读到的入站帧（不透明字节 + 路由 hint）
    PortReply(PortReply),           // effect 的回报（带 correlation id + ok/err + payload）
    TimerFired { id: TimerId },     // Clock 到点
    AppCommand(AppCommand),         // host/UI 下发的用户意图（不透明）
}

pub struct StepInput {
    pub tick: Tick,
    pub now_ms: u64,                // host 注入的确定性时间（§Clock）
}

impl CoreEngine {
    /// 纯函数语义：同一 (StepInput 序列) 必产出同一 (Vec<Effect> 序列)。
    /// 零 I/O / 零 await / 零 spawn —— 只算，只吐 effect。
    pub fn step(&mut self, input: StepInput) -> Vec<Effect> { /* dispatch 给 Module */ }
}
```

`Effect` 是 core 定义的**抽象信封**，core 不认识 variant 内容（§1 文档 §3.2 已列）：

```rust
// 【core 通用】helix-core/src/effect.rs
pub struct Correlation(pub u64);    // core 分配的关联号；模块用它把回报配对到 pending 状态

pub enum Effect {
    Send      { transport: TransportId, frame: OutboundFrame },     // → Transport port
    Persist   { corr: Correlation, ops: Vec<StorageOp> },           // → Storage port（带 corr）
    Http      { corr: Correlation, req: HttpRequest },              // → HttpRequester port（带 corr）
    Emit      { event: DomainEvent },                               // → EventSink port
    ScheduleTimer { id: TimerId, after_ms: u64 },                   // → Clock port
    CancelTimer   { id: TimerId },                                  // → Clock port
}
```

> 关键：`Persist` / `Http` 携带 `corr`；`PortReply { corr, ok, payload }` 回流时，模块用 `corr` 找到自己的 pending 状态推进——这是 **peek/commit 写成功才推 cursor** 的 core 级地基（§5 详述）。`OutboundFrame` / `StorageOp` / `HttpRequest` / `DomainEvent` 内部全是模块塞的不透明内容。

---

## 1. IM 模块如何实现 core 的 `Module` trait

### 1.1 Module trait（core 定义，IM 实现）

```rust
// 【core 通用】helix-core/src/module_host.rs
pub trait Module: 'static {
    fn name(&self) -> &'static str;

    /// 帧/命令是否归我处理（路由判定）。core 据此 dispatch。
    fn accepts(&self, frame: &InboundFrame) -> bool;
    fn accepts_command(&self, cmd: &AppCommand) -> bool;

    /// 模块挂载时调用一次。拿到 port 句柄 + 配置；可吐初始 effect（如发 hello / 装载本地 cursor）。
    fn init(&mut self, ctx: &mut InitContext) -> Result<(), CoreError>;

    /// 主处理：一个 Tick 进来 → 纯逻辑 → 吐 Vec<Effect>。零 I/O。
    fn handle(&mut self, tick: &Tick, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError>;

    /// 周期 tick（由 host 的稳定节拍喂入，now_ms 注入）。
    /// 用于"无外部事件也要推进"的逻辑：心跳、退避计时收敛、超时扫描兜底。
    fn tick(&mut self, now_ms: u64, out: &mut EffectSink) -> Result<(), CoreError>;

    /// 卸载/关闭。吐收尾 effect（flush pending、close 连接、撤所有 timer）。
    fn drop(&mut self, out: &mut EffectSink) -> Result<(), CoreError>;
}
```

> `EffectSink` 是 `01` 文档 §6.3 的 `EffectContext` 的收口写法；`InitContext` 给模块**只读的 port 能力描述符 + 配置**，**不给真实 I/O 句柄**（真实句柄在 driver；模块只吐 effect）。

### 1.2 IM 对四个方法各做什么

| 方法 | IM 做什么（全部来自源码事实，全部纯逻辑/吐 effect） | 归属 |
|---|---|---|
| **`init`** | ① 从 Storage 读回 per-channel cursor（吐 `Persist{get channel_event_cursor}`，回报到达后填 `gate_states`）；② 吐 `Send{ WS hello 帧 }` 发起握手（源码 `hello` 帧）；③ 注册需要的 timer 槽位（不立即起，超时按需起）。**`init` 只吐 effect，不阻塞、不连接**——真正连 socket 是 driver 收到首个 `Send` 时做的事。 | 【IM 专属】hello 帧/cursor 表名是 IM 内容；【core 必备】init 时机 + 吐 effect 通道是 core |
| **`handle`** | 全部 sans-IO 状态机（见 §1.3）：解析 14 类 WS 帧 → gate seq 判定 → 双 flush → 对账 by temporaryId → 分桶顺序 → peek/commit。输入 = `Tick`（Inbound 帧 / PortReply / TimerFired / AppCommand send·read·resend）；输出 = `Effect` 流。 | 【IM 专属】帧解析/seq/对账逻辑；【core 必备】`handle(tick,now_ms,out)` 签名 |
| **`tick`** | ① 心跳：到 8s 吐 `Send{ ping }`，记录 `last_ping_at`（详见 §3.4，这是检验 core 时补的能力）；② 退避收敛：重连退避计时纯推进；③ **超时兜底**：扫 pending send，`now_ms - sent_at >= 15000` 的吐 `Persist{ update sendStatus=unSend }`（与 `ScheduleTimer` 双保险，源码主路径是 timer，tick 是兜底防 timer 漏）。tick 是"无外部事件也要推进时间相关状态"的入口。 | 【IM 专属】8s/15s 数值、ping 帧；【core 必备】周期 tick 注入 now_ms 是 core |
| **`drop`** | ① `flush pending`：把内存 buffer 里已收齐但未落库的 event 吐最后一批 `Persist`；② 吐 `CancelTimer` 撤掉所有 gate/超时 timer；③ 吐 `Send{ WS close }`。**幂等**：drop 后再喂 tick 不产生 effect。 | 【IM 专属】close 帧；【core 必备】drop 生命周期钩子 |

### 1.3 IM 状态机如何活在 sans-IO 模型

IM 的全部可变状态**都在 `helix-im` 模块结构体内**，没有一个字段是 tokio/sqlx/socket 句柄：

```rust
// 【IM 专属】helix-im/src/state.rs —— 纯数据，可序列化、可回放、可快照对比
pub struct ImModule {
    conn: ConnState,                                   // Connecting/Open/Closing/Closed + 退避计时（纯枚举，§3.4）
    gates: HashMap<ChannelKey, ChannelGateState>,      // per-channel: last_cursor + 乱序 buffer + gate timer id
    pending_sends: HashMap<TempId, PendingSend>,       // temporaryId → {sent_at, timer_id}（对账/超时用）
    pending_commits: HashMap<Correlation, CommitPlan>, // peek 阶段算好的 target_seq，等 PersistResult（§5）
    next_corr: u64,                                    // 模块自管的 correlation 递增（也可由 core 分配，见 §3）
    cfg: ImConfig,                                     // 15000ms / 1000ms / 8000ms / shard_count=4 等业务常量
}

struct ChannelGateState {
    last_cursor: u64,                    // 已确认连续推进到的 event_seq
    buffer: BTreeMap<u64, ImEvent>,      // seq > cursor+1 的乱序事件（BTreeMap 保序，flush 时遍历）
    gate_timer: Option<TimerId>,         // 1s gate timer（None=未起）
}
```

**为什么这就是 sans-IO**：所有判定（seq 比较、buffer 插入/取出、temporaryId 匹配、分桶排序、退避计时）都是对上面纯数据结构的读写，**没有 await、没有 I/O、没有读时钟**（时间从 `now_ms` 参数进来）。给同一 Tick 序列 + 同一 now_ms 序列，状态机演进与 effect 输出**逐字节确定**——这正是三端一致 + 可回放 + 可单元测试的根。

源码里 `post.rs` 把 `app.emit` / `sqlx upsert` / `tokio::spawn` 和 seq 判定混在一个函数里；Helix 的做法是**只搬走 seq 判定那部分逻辑进 `handle`，I/O 部分变成它吐出的 `Effect`**。

### 1.4 4-worker hash 分片在 sans-IO 下怎么表达

源码事实：`message_loop.rs` dispatcher 按 `hash(channel_id) % 4` 把帧分到 4 个 worker，**目的是 per-channel 串行 + 跨 channel 并行**（同 channel 的 seq gate 必须串行，否则 cursor 推进竞态）。

sans-IO 下的裁决：**分片不是并发，是"按 channel 路由到独立状态子机"**。

```rust
// 【IM 专属】分片纯逻辑在 IM 内部，不是 core 概念
fn shard_of(ch: &ChannelKey) -> u8 { (stable_hash(ch) % self.cfg.shard_count) as u8 }
```

- **core 单线程喂 Tick（wasm）**：4-worker 退化为"按 channel key 路由到 `gates` 里对应的 `ChannelGateState`"——`HashMap<ChannelKey, _>` 本身就保证了 per-channel 串行（同 key 同一份状态，顺序处理）。**分片在 wasm 上是逻辑分区，不需要线程。**
- **core 多线程喂 Tick（native）**：driver 层可以开 4 个 executor task，**每个 task 持有一个 `CoreEngine` 分片实例**（或一个 Engine 内按 shard 加锁），保证同 channel 的 Tick 永远进同一分片串行处理。**这是 driver 的并发决策，core 只保证"同 channel Tick 串行喂入同一 Module 实例"这个契约**。

> **关键区分**：
> - 【core 通用必备】= "同一逻辑分区的 Tick 串行喂入" 的契约（core 提供按 routing-key 串行的 dispatch 保证）。
> - 【IM 专属·不能进 core】= "分 4 片 / hash(channel_id)%4 / channel 作为分区键" —— 4 这个数字、channel 作分区键都是 IM 的，core 只知道有个 `routing_key: PartitionKey`（不透明），保证同 key 串行。
> - **并发模型绝不 bake 进 core**：4 个线程是 native driver 的事；wasm driver 单线程跑同一份 IM 逻辑，行为完全一致（因为分片是逻辑分区不是并发）。这正是 `01` 文档教训的落地。

### 1.5 1s gate timer 在 sans-IO 下怎么表达

源码事实：seq > cursor+1 → buffer + 起 1s timer；1s 到 `on_gate_fire` 异步 `tokio::spawn` 发 HTTP sync notify；seq 补齐则 cancel gate timer。

sans-IO 表达（**timer = effect，fire = 入站事件**，与 §0 一致）：

| 源码动作 | sans-IO effect / tick |
|---|---|
| seq > cursor+1，起 1s gate timer | `handle` 吐 `ScheduleTimer{ id=gate_timer_id, after_ms=1000 }`，存进 `ChannelGateState.gate_timer` |
| seq 补齐（收到缺失的 seq），cancel gate | `handle` 吐 `CancelTimer{ id=gate_timer_id }`，清 `gate_timer=None` |
| 1s 到，`on_gate_fire` 发 sync | driver 喂 `TimerFired{ id=gate_timer_id }` → `handle` 命中 → 吐 `Http{ corr, req=/sync/notify?from_seq=cursor }` |
| `on_gate_fire` 里的 `tokio::spawn` | **删掉**。spawn 是并发，不进 core。HTTP 由 driver 在自己 executor 上异步发，响应作 `PortReply` 喂回。模块只吐 `Http` effect 不 spawn。 |

> 1000ms 这个数字、"gate"概念、"sync notify"语义全是【IM 专属】；【core 通用必备】只是 `ScheduleTimer{id, after_ms}` / `CancelTimer{id}` / `TimerFired{id}` 这组通用定时原语 + "timer 由外部时钟兑现后作入站事件回流" 的 sans-IO 约定。

---

## 2. 挂载流程完整时序

```
┌─ host 进程（Tauri main / mobile FFI host / browser JS）
│
│ ① 选 driver（编译期 cfg + 运行期实例化）
│    native: HelixDriverNative { tokio rt, sqlx pool, tungstenite, reqwest, SystemTime clock }
│    wasm:   HelixDriverWasm   { spawn_local, sqlite-wasm-rs/OPFS, web-sys WS, fetch, Date.now clock }
│    ffi:    HelixDriverFfi    { host 回调注入: send_fn/query_fn/now_fn/http_fn }
│
│ ② driver 实现 core 的 6 个 port trait（Transport/Storage/Clock/EventSink/Id+Random/HttpRequester）
│    —— 这是 driver 唯一职责：把 core 的 Effect 兑现成真实 I/O，把真实 I/O 结果包成 Tick 喂回。
│
│ ③ 建 CoreEngine：let mut engine = CoreEngine::new(ports);
│    core 内部持有 ModuleHost（空）+ EffectContext + correlation 分配器。
│
│ ④ 注册 IM 模块：engine.register(ImModule::new(im_config));
│    ModuleHost.modules.push(Box::new(im));  ← 只是入表，未 init。
│
│ ⑤ engine.start():
│    对每个已注册 module 调 module.init(&mut init_ctx)；
│    IM.init 吐出初始 effect: [Persist{get cursor}, Send{hello}, ...]
│    core 把这些 effect 收集 → 交 driver 执行。
│
└─ ⑥ 进入运行循环（driver 侧，core 之外）：
     loop {
        // a. driver 异步等待任一来源就绪（WS 帧 / 定时器 / port 回报 / UI 命令）
        let raw = driver.next_event().await;       // ← 唯一 await 在 driver
        // b. driver 把 raw 包成 Tick + 注入 now_ms
        let input = StepInput { tick: driver.to_tick(raw), now_ms: clock.now_ms() };
        // c. 喂入 core 纯函数 step（无 await）
        let effects = engine.step(input);          // ← core 纯逻辑，零 I/O
        // d. driver 兑现 effect（真实 I/O；带 corr 的把回报排队成下一个 Tick）
        for e in effects { driver.execute(e).await; }  // execute 内部异步，完成时把 PortReply 入队
     }
```

**时序要点（区分归属）**：

- **【core 必备】** ③④⑤ 的 `register` / `start` / `step` 是 core 通用 API，任何模块都走这条路，core 不认识 IM。
- **【core 必备】** port 注入方向：**driver → core**（driver 实现 trait，构造 Engine 时传入）。模块**反向只看 port 能力描述**，不持有 driver 句柄——依赖方向单向（driver 依赖 core 依赖模块契约，模块不依赖 driver）。
- **【IM 专属】** ⑤ 里 IM.init 吐 `Send{hello}` / 读 cursor 表 —— hello 协议、cursor 表名是 IM 的。
- **并发模型在 ⑥**（driver 的 loop / 多 task），**完全在 core 之外**。wasm 是单 `spawn_local` loop，native 是 tokio `select!` 多路，ffi 是回调驱动。三者喂给 `engine.step` 的都是同样的 `StepInput`，core 行为一致。

---

## 3. 实时核通用性检验：四把尺

> 用真正实时系统（Centrifugo/centrifuge 级）的四个硬需求检验 helix-core 的 6 port + 执行壳够不够。每把尺给出：core 现状是否覆盖、缺什么、多什么。

### 3.1 尺一：Transport 抽象（多协议）

**实时核要求**：协议层只跟 Transport interface 对话，不知道下层是 WS / SSE / HTTP-stream / WebTransport / GRPC；增加新 transport ≈ 实现一个 interface。

**core 现状**：`Transport` port 搬不透明 `OutboundFrame`，入站由 driver 读出包成 `Tick::Inbound`。IM 只用 WS，但 trait 本身不绑 WS。

**裁决：覆盖，但要补一处。**

- ✅ 不透明帧 + driver 实现 trait = 标准 ports & adapters，加新 transport 不改 core/模块。
- ⚠️ **缺：多 transport 并存的寻址**。`01` 文档已识别 IM 有 Transport(WS) + HttpRequester(HTTP) 双路径。实时核进一步要求"一个模块可能同时挂多条 transport"（如主 WS + SSE 降级 + emulation HTTP 上行）。`Effect::Send { transport: TransportId, .. }` 里的 `TransportId` 就是为此预留——**core 必备**：Effect 的 Send/入站 Tick 都要带 `TransportId`，让模块能寻址"发哪条 / 这帧来自哪条"。
- ⚠️ **缺：transport 生命周期事件入站**。实时核需要 `connected / disconnected / error` 作为入站事件（重连状态机要据此推进，§3.4）。**core 必备**：`Tick` 要加 `TransportEvent { id, kind: Opened|Closed|Error }`。当前 §0 的 Tick 没有它——**这是检验补出来的 core 增量**。
- **不多**：core 不该认识 SSE/WebTransport 具体协议（那是 driver 的）。

### 3.2 尺二：recovery / cursor（对照 IM 的 per-channel cursor + 双 flush + too_long）

**实时核要求**：客户端重连后能从断点恢复（offset/epoch 二维 recovery）；丢太多则 fallback 整体重拉。

**core 现状**：IM 已实现 per-channel cursor（`cursor_upsert` 单调）+ flush_contiguous(WS 严格+1) / flush_up_to(sync 跳空洞) + TooLong 兜底。这些**全在 helix-im 内核**，core 只提供 `cursor_upsert(scope_key, value)` 单调原语 + correlation 配对。

**裁决：覆盖，且这是 core 通用性的最强证据。**

- ✅ **cursor 模型本身就是 IM 私有，不进 core**——core 只给"单调写 + 写成功才推"地基（§5）。第二个模块若要自己的 recovery（如 docs 模块的版本号），用同一个 `cursor_upsert` 原语，scope_key 换成 doc_id，core 一行不改。
- ✅ **epoch（断点失效检测）**：实时核的 epoch 是"broker 重启则 offset 作废"。IM 的 TooLong 就是同语义（`reset_to` → 清表重拉）。core 不需要认识 epoch/TooLong——它是模块在 PortReply（sync 4 kind）里自己判的纯逻辑。
- ⚠️ **多一点点（可接受）**：core 的 `cursor_upsert` MAX guard 语义有点"偏 recovery"。但它本质是"单调 KV 写"通用原语，notify 模块的"已读水位"也能用，所以不算污染，保留。
- **缺什么：无。** recovery 的全部花样（双 flush / TooLong / 续拉）都能在模块内核用 §0 的 Tick/Effect 表达。这把尺证明 **core 的 cursor 契约抽象层级正确**（给原语不给策略）。

### 3.3 尺三：背压 / 慢消费（slow consumer）

**实时核要求**：服务端给每个连接一个发送队列（按字节计），满了**必须 kick** 整个连接，不能拖垮整个节点（Centrifugo 不变量）。客户端侧对称：本地处理不过来时要有背压信号。

**core 现状**：`Effect::Send` 是 fire-and-forget，**没有背压回路**。`futures-channel` 在 driver 侧有 unbounded 风险。

**裁决：部分覆盖，缺背压契约——这是检验出的最大 core 缺口。**

- ❌ **缺：Send 的可发性回报 / 队列水位入站**。客户端实时核里，若 WS 出站缓冲堆积（网络慢），模块需要知道"发不动了"以决定丢弃/合并/降级。**core 必备增量**：
  - `Effect::Send` 升级为可携带 `corr`（可选），让模块对关键帧拿到"已写入 transport / backpressure / 失败"回报（`PortReply`）。
  - 或 `Tick::TransportEvent` 增加 `kind: Drained | Backpressure { queued_bytes }`，driver 在出站队列越过阈值时喂入，模块在 `handle`/`tick` 里据此降速。
- ❌ **缺：入站背压（慢消费的客户端版）**。若 Storage/EventSink 兑现慢（OPFS 写盘慢、UI 线程忙），core 必须能限制"未完成 effect 的在途数"。**core 必备**：执行壳应支持 **bounded in-flight effects**——driver 可声明"同时最多 N 个未回报 Persist"，超了 core 暂缓 dispatch 新 Tick（或 Tick 排队）。这把背压收进 core 的 dispatch 调度，而不是让每个模块自己造队列。
- ⚠️ **kick 语义归属**：实时核服务端"踢连接"是 server 行为。Helix 是客户端核，对称行为是"主动 close + 重连"——这是 IM 模块用 `Effect::Send{close}` + 重连状态机表达的【IM 专属】策略，**不进 core**。core 只提供 close effect + transport 生命周期事件。
- **结论**：背压是 core **当前缺失**的通用契约，必须补：① Send 可选 corr 回报 ② TransportEvent 带队列水位 ③ 执行壳 bounded in-flight。**这三条是通用的**（任何实时模块都需要），所以进 core 不算污染。

### 3.4 尺四：心跳 / 重连（8s ping / 5s timeout / 指数退避）

**实时核要求**：客户端定期 ping，超时未 pong 判定半死连接 → 主动断 → 指数退避重连（带 jitter）。

**core 现状**：IM 源码有 hello 握手；ping/pong + 退避在 driver 层（cses-client 是 WsRuntime 状态机）。问题：**重连状态机该在 core、driver 还是模块?**

**裁决：覆盖，归属要钉死——这正是"通用 vs 专属"最容易划错的地方。**

- **重连状态机 = 【IM 专属·在模块内核】**，不是 core，也不应该埋在 driver。理由：
  - 它是**纯逻辑状态机**（Connecting/Open/Closing/Closed + 退避计时 + ping 间隔），完美 sans-IO：
    ```rust
    // 【IM 专属】helix-im/src/conn.rs —— 纯状态机
    enum ConnState {
        Closed { retry_at_ms: u64, backoff_ms: u64 },   // 退避到点重连
        Connecting { since_ms: u64 },
        Open { last_ping_ms: u64, last_pong_ms: u64 },
        Closing,
    }
    ```
  - `tick(now_ms)` 驱动它：`Open` 且 `now - last_ping >= 8000` → 吐 `Send{ping}`；`now - last_pong >= 5000`（5s timeout）→ 吐 `Send{close}` + 转 `Closed{retry_at, backoff*2 + jitter}`；`Closed` 且 `now >= retry_at` → 吐 `Send{reconnect/hello}` + 转 `Connecting`。
  - **退避指数 + jitter 必须确定性**：jitter 的随机数从 `Random` port 注入（host 注入种子），**不在内核调 rand**——这样重连序列可回放。这正是 `Id/Random` port 存在的真实用途之一（`01` 文档 §1.5 说 IM 主路径不生成 id，但**重连 jitter 是 Random port 的真实消费者**——补充了 port 的存在理由）。
  - 把它放模块而非 driver 的好处：**三端重连行为逐字节一致**（同样的 now_ms + 同样的 jitter 种子 → 同样的退避序列），可单测。若放 driver，native/wasm/ffi 各写一份，必然漂移。
- **【core 必备】支撑项**（这把尺要求 core 提供的）：
  - `tick(now_ms)` 周期注入 —— ✅ §1.1 已有。
  - `TransportEvent { Opened/Closed/Error }` 入站 —— ⚠️ §3.1 已标为缺，**重连状态机依赖它**（socket 真断了 driver 要喂 Closed，模块才转 `Closed{retry}`）。再次确认这是 core 必补增量。
  - `Random` port 注入 jitter 种子 —— ✅ 已有。
- **8000/5000/退避基数 = 【IM 专属】** 业务常量，在 `ImConfig`，不进 core。

### 3.5 四把尺汇总：core 契约缺什么 / 多什么

| 尺 | 覆盖度 | core 必补增量（通用，不污染） | 确认无需进 core 的（模块/driver 专属） |
|---|---|---|---|
| Transport 抽象 | 基本覆盖 | `TransportId` 寻址；`Tick::TransportEvent{Opened/Closed/Error}` | SSE/WebTransport 具体协议 |
| recovery/cursor | **完全覆盖** | 无（`cursor_upsert` 单调原语已够） | 双 flush / TooLong / epoch / 续拉（全模块内核） |
| 背压/慢消费 | **缺** | ① `Send` 可选 corr 回报 ② `TransportEvent` 带 `Backpressure{queued_bytes}/Drained` ③ 执行壳 **bounded in-flight effects** 调度 | kick/close 策略（IM 用 close effect 表达） |
| 心跳/重连 | 覆盖（靠 tick+Random+TransportEvent） | `TransportEvent`（与尺一同款）；其余已有 | 重连状态机 / 8s·5s·退避数值（全 IM 模块内核） |

**净增 core 契约（本次检验产出，全部通用）**：
1. `Tick::TransportEvent { id: TransportId, kind: Opened | Closed | Error | Backpressure{queued_bytes} | Drained }` —— transport 生命周期 + 背压信号入站。
2. `Effect::Send` 增 `transport: TransportId` 寻址 + 可选 `corr` 拿可发性回报。
3. 执行壳 **bounded in-flight effects** 调度（driver 声明上限，core 据此节流 dispatch）。

**多出来的（要警惕但本次没有）**：core 没有混进任何 IM 概念。`cursor_upsert` 的 MAX guard 是唯一"略偏 recovery"的原语，但它是通用单调写，保留。

**结论**：6 个 port 抽象层级正确，执行壳 sans-IO 形状正确；**唯一系统性缺口是背压/transport 生命周期**，补 3 条通用契约即可覆盖 Centrifugo 级实时核需求。recovery 这把尺反过来证明 core 抽象正确（给原语不给策略）。

---

## 4. 第二模块设想：presence（证明 core 不与 IM 绑定）

设想一个 **presence 模块**（在线状态 / 谁在看这个 channel），跟 IM 并存，**用完全相同的 6 port + 同一执行壳挂上去，core 一行不改**。

### 4.1 presence 用同一套 port

| port | presence 怎么用（与 IM 不同的内容，相同的契约） |
|---|---|
| `Transport` | 同一条 WS（或自己的 transport）。订阅 presence 帧（`presence_join/leave/heartbeat`——presence 私有 wire 名，core 不认识，正如 core 不认识 IM 的 `post`）。`accepts(frame)` 按帧 type 前缀区分归 presence 还是 IM。 |
| `Storage` | `set(table="presence", key=channel:user, value=online_at)` + `cursor_upsert(scope_key=channel, value=presence_epoch)`——**复用 IM 同款 cursor 原语**做 presence 的 recovery，core 完全不知道这次 scope 是 presence。 |
| `Clock` | `ScheduleTimer{ heartbeat 25s }`——自己起心跳；`TimerFired` 到点吐 `Send{heartbeat}`。**与 IM 的 1s gate / 15s 超时用同一组 timer 原语**，TimerId 各自管。 |
| `EventSink` | `Emit{ DomainEvent::PresenceChanged{...} }`——presence 自己的 DomainEvent variant；driver 序列化为 `presence:changed` wire 名。**core 的 EventSink 不知道有 `im:` 还是 `presence:` 前缀。** |
| `HttpRequester` | presence 全量快照拉取 `Http{ req=/presence/snapshot }`——自己的 URL，core 搬不透明 request。 |
| `Id/Random` | presence 也可能要 jitter 退避——同 Random port。 |

### 4.2 两个模块如何并存

```rust
// host 侧（driver 之上）
let mut engine = CoreEngine::new(ports);
engine.register(ImModule::new(im_cfg));         // 模块 1
engine.register(PresenceModule::new(pres_cfg)); // 模块 2 —— core 不需要任何改动
engine.start();
```

- **路由**：`ModuleHost.dispatch` 遍历模块 `accepts(frame)`，按帧 type 分派给 IM 或 presence。两模块各自维护独立状态（`gates` vs `presence_table`），互不可见。
- **共享一条 WS**：driver 把一条 WS 的所有入站帧包成 `Tick::Inbound`，core 按 `accepts` 路由。出站时 `Effect::Send{transport}` 用同一 `TransportId`。**这正是 §3.1 `TransportId` 寻址的价值**——多模块共享/区分 transport。
- **生命周期独立**：`engine.start()` 对两模块各调 `init`；`tick(now_ms)` 同时驱动两者的时间逻辑（IM 心跳 + presence 心跳互不干扰，各自 TimerId）。
- **零耦合证明**：删掉 `helix-im`，presence 仍跑（只要它不 import helix-im）；core 编译通过、grep 不到 `presence:` 也不到 `im:`。**这就是"加新业务模块 = 实现 Module 契约 + 注册，不改 core"**（质量轴 4）。

### 4.3 这验证了什么

- presence 与 IM **共用** cursor 原语、timer 原语、transport、Storage KV、EventSink——证明 `01` 文档 §1 抽象的 6 port **不是为 IM 量身定做**，是通用实时模块的最小公倍数。
- presence **不需要** peek/commit 写成功才推（它的状态是 last-writer-wins，不是事件流对账）——证明 core 的 correlation 契约是**可选能力**（IM 用，presence 可不用），core 不强加。
- 若将来加 docs 协作模块（OT/CRDT），它会**重度复用** correlation 配对（每个 op 落库成功才推版本号）——证明 §5 的 correlation 契约对"第二类需要事务式推进的模块"也通用，不是 IM 一次性需求。

---

## 5. C006 / temporaryId / "写成功才推 cursor" 在该模型下的落点

> 三者都在 `01` 文档 §5/§6 定义过；这里给"在 Module/Tick/Effect 模型里**具体落在哪个方法、哪个 effect、哪个 tick**"。

### 5.1 temporaryId：落在 AppCommand 入站 + Persist 的 conflict_key 参数

- **落点**：用户发消息 = `Tick::AppCommand(AppCommand::Send{ data, temporary_id })`（temporaryId **由 host/UI 注入随命令带进来**，源码印证 `messageSend` 主路径）。
- IM 的 `handle` 收到后吐：
  - `Effect::Persist { corr, ops: [StorageOp::BatchUpsert { table="message", rows:[乐观行 sendStatus=local], conflict_key="temporary_id" }] }`
  - `Effect::ScheduleTimer { id=TimerId(temp_id), after_ms=15000 }`
  - 记 `pending_sends[temp_id] = { sent_at: now_ms, timer_id }`
- **归属**：`conflict_key="temporary_id"` 是 `Persist` 的**字符串参数**，core 不认识（`01` 文档红线）。temporaryId 作为对账 PK 的语义 100% 在 `helix-im`。

### 5.2 对账（echo/sync 同一对账点）：落在 handle 的同一纯函数

- **落点**：实时 echo 入站（`Tick::Inbound(post 帧带 server id)`）与 sync 入站（`Tick::PortReply(sync Events)`）**进 `handle` 后归并到同一个 IM 内部纯函数 `reconcile(server_id, temp_id)`**（源码：两者同走 `im:post:received` handler）。
- 该函数吐**完全相同的 effect 序列**（这就是"同一对账点"在模型里的形态）：
  - `Effect::Persist { ops: [BatchUpsert { rows:[server id 值级覆盖, sendStatus=sent], conflict_key="temporary_id" }] }` —— 对账落地 = upsert 覆盖（`ON CONFLICT id=excluded.id`），不是新建第二行。
  - `Effect::CancelTimer { id=TimerId(temp_id) }` —— 撤 15s 超时（源码 `clearSendTimeout`）。
  - `Effect::Emit { DomainEvent::PostReceived }`。
  - 清 `pending_sends[temp_id]`。
- **归属**：reconcile 纯逻辑、值级覆盖语义全在 IM；core 只提供"两个入站来源（Inbound / PortReply）都能进同一 handle"——这是 sans-IO 执行壳的天然能力。

### 5.3 C006（补发时序）：落在 TimerFired/AppCommand 入站次序 + 去重纯逻辑

C006 = 补发时序的确定性收敛。两种到达次序，**同一终态**：

| 次序 | 入站事件序列 | handle 产出 | 终态 |
|---|---|---|---|
| echo 先到 | `Inbound(echo)` → 后续 `TimerFired(15s)` | echo: `CancelTimer` + upsert sent；TimerFired: **找不到 pending_sends[temp_id]（已清）→ 不产出 effect** | sent（15s 永不生效） |
| timer 先到 | `TimerFired(15s)` → 后续 `Inbound(echo)` | TimerFired: `Persist{update sendStatus=unSend}`；echo: **仍按 temporary_id upsert 覆盖回 sent** | sent（迟到 echo 仍对账成功） |

- **落点**：`handle` 对 `TimerFired` 的处理**先查 `pending_sends`**（去重门），对 `Inbound(echo)` 的处理**总按 temporary_id upsert**（幂等覆盖）。两个守卫保证**任意到达次序收敛同一终态**——这是 C006 在状态机里的体现。
- **无自动补发**：`TimerFired(15s)` 只吐 `update unSend`，**绝不吐重试 effect**（源码 2026-05-30 删第二套状态机）。重发只能由 `Tick::AppCommand(Resend)` 触发（用户手点），重走 §5.1。
- **归属**：15000ms、unSend 取值、"手点重发"全是【IM 专属】；core 只提供 `TimerFired` 入站 + `CancelTimer` effect + "去重靠模块内核纯逻辑"——core 不认识 C006。

### 5.4 "写成功才推 cursor"：落在两段式 handle + Persist 的 corr + PortReply

这是 §0 correlation 契约的**唯一硬性 core 增量需求**的真实消费者。

**Step N（收到 sync Events 入站 `Tick::PortReply(Events)`）** → IM `handle`：
- `peek_up_to` 纯逻辑：分桶 type1/2/3/6（顺序铁律）算出"该写哪些" + "若全成功 cursor 推到 `target_seq`"。
- 吐 `Effect::Persist { corr=C, ops:[分桶批] }`（**带 core 分配的 `corr`**），**此刻不吐 cursor_upsert**。
- 记 `pending_commits[C] = CommitPlan { scope_key=channel, target_seq }`。

**Step N+1（收到 `Tick::PortReply(PersistResult{ corr:C, ok })`）** → IM `handle`：
- `ok==true` → `commit_up_to`：吐 `Effect::Persist { ops:[StorageOp::CursorUpsert { scope_key=channel, value=target_seq }] }`（MAX guard 单调），内存 cursor 推到 target_seq，清 `pending_commits[C]`。
- `ok==false` → **不推 cursor**，丢弃/保留 plan，下次 sync 仍从旧 cursor 拉（不丢事件）。

**确定性保证**：cursor 推进**只锚定在 `PersistResult` 这个确定性入站事件**上，不依赖时钟/线程时序 → 同一 (Events 入站 + PersistResult 入站) 序列必产同一 cursor 终值，可回放、三端一致。

**core 必备 vs IM 专属**：
- 【core 必备】= `Persist`/`Http` 带 `corr`；`PortReply{corr, ok, payload}` 回流；模块能在内核保存 `pending_commits` 跨 step（执行壳允许模块持有跨 step 状态——天然成立，因为 Module 是 `&mut self`）。这三条对任何"事务式推进"模块（IM cursor / 未来 docs 版本号）通用。
- 【IM 专属】= peek/commit 命名、分桶顺序铁律、`channel_event_cursor` 表、MAX guard 为 event_seq、flush_contiguous vs flush_up_to 的 target_seq 计算规则——全在 helix-im 内核。

---

## 6. 全文归属总表（core 通用必备 vs IM 专属·不能进 core）

| 主题 | 【core 通用必备】 | 【IM 专属·不能进 core】 |
|---|---|---|
| 模块契约 | `Module` trait（init/handle/tick/drop）+ `accepts` 路由 | ImModule 状态结构、14 类帧解析 |
| 执行壳 | `step(StepInput)->Vec<Effect>` 纯函数；`Tick` / `Effect` 信封 | 每个 variant 内的 IM 内容 |
| 分片 | "同 routing-key Tick 串行喂入同一模块"契约；`PartitionKey` 不透明 | hash(channel_id)%4、4 这个数、channel 作分区键 |
| timer | `ScheduleTimer/CancelTimer/TimerFired` 通用原语 | 1s gate / 15s 超时 / 8s ping / 5s timeout 数值 |
| transport | Transport port + `TransportId` 寻址 + `TransportEvent` 生命周期/背压入站 | WS 协议、hello/post/ping 帧格式 |
| cursor/recovery | `cursor_upsert(scope_key,value)` 单调 MAX guard 原语 | per-channel、event_seq、双 flush、TooLong、续拉 |
| 背压 | Send 可选 corr 回报 + bounded in-flight 调度 | close+重连降级策略 |
| 重连 | tick(now_ms) + TransportEvent + Random 种子注入 | ConnState 状态机、退避基数、jitter 算法 |
| 对账 | 多入站来源进同一 handle + Persist conflict_key 参数化 | temporaryId 作 PK、值级覆盖、reconcile 逻辑 |
| 写成功才推 | `corr` + `PortReply{corr,ok}` 配对 + 跨 step pending 状态 | peek/commit、分桶顺序、commit_up_to 规则 |
| 事件出口 | `EventSink::emit(DomainEvent)` | `im:` 前缀、`im:__bus__`、8 族事件 wire 名 |

**一句话验收**：删掉 `helix-im`，`helix-core` 必须仍能编译，且 grep 不到本表右列任一词汇；同时 `helix-presence`（§4）能在不改 core 的前提下用同一套左列契约挂上。

---

## 7. 本文产出的 core 契约增量（汇总给后续骨架实现）

相对 `01` 文档（已确认 6 port + step + correlation 配对），本文经四把实时核尺**新增 3 条通用 core 契约**：

1. **`Tick::TransportEvent { id: TransportId, kind: Opened | Closed | Error | Backpressure { queued_bytes } | Drained }`** —— transport 生命周期 + 背压信号入站（尺一/三/四共同需要）。
2. **`Effect::Send { transport: TransportId, frame, corr: Option<Correlation> }`** —— 多 transport 寻址 + 关键帧可发性回报（尺一/三）。
3. **执行壳 bounded in-flight effects 调度** —— driver 声明在途 effect 上限，core 据此节流 dispatch，把背压收进调度而非每模块自造队列（尺三）。

这 3 条全部通用（presence/docs 同样受益），不引入任何 IM 概念，符合"够用 + 不污染"判据。
