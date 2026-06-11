# Helix · Rust Core 技术过会（图驱动版）

> 第一人称 · 2026-06-09 · 围绕**技术选型 / 架构设计 / 流程图**，不堆文字。
> 事实基准：`helix-core` 零运行时实测绿 + 三栖编译绿 + 7 条 HX-C 护栏。

---

## §0 原点（一句话量这次过会）

> **Helix = 运行时无关的确定性业务内核**：业务逻辑写一次，编译到三端（native/Tauri · wasm/浏览器 · FFI/移动），行为一致、可回放可测试；core 不认识任何业务，IM 是第一个 guest 模块。

要解决的三个核心问题 → 这次每个技术选型都回答「它为哪个问题打地基」：

| | 核心问题 | 现状的痛 |
|---|---|---|
| **P1** | 业务逻辑写三份 | 三端各写 IM ≈2550 行，维护三份 |
| **P2** | Bug 修三处、行为分叉 | cursor Bug native 修完，FFI/wasm **后续**才同步 |
| **P3** | 业务逻辑测不动 | 测试依赖真实平台，覆盖率 <40%，时序 Bug 难复现 |

判据：**是为原点构建的框架，不是打补丁。** 下面用图说话。

---

## §1 架构设计（一张图看依赖方向）

```mermaid
flowchart TB
    subgraph L1["业务模块层（确定性状态机 · 零运行时）"]
        IM["helix-im<br/>消息/游标/对账/同步"]
        PR["helix-presence（将来 · 不改 core 挂载）"]
    end
    subgraph L2["core layer · helix-core （产品本体）"]
        SH["ExecutionShell<br/>step(Tick, now_ms) → &[Effect]<br/>同步纯函数 · 零 await/IO/spawn"]
        PORTS["5 类 Port · 7 trait: Transport · Storage · HttpRequester · EventSink · Clock/IdSource/Random"]
    end
    subgraph L3["平台驱动层（每端各一 · core 之外）"]
        DN["helix-driver-native<br/>tokio·sqlx·tungstenite·reqwest"]
        DW["helix-driver-wasm<br/>wasm-bindgen·web-sys·OPFS"]
        DF["helix-driver-ffi<br/>C ABI·uniffi·rusqlite"]
    end
    IM -->|实现 Module trait| L2
    PR -->|register 一行| L2
    DN -->|实现 ports| L2
    DW -->|实现 ports| L2
    DF -->|实现 ports| L2
    style L2 fill:#1a3a5c,color:#fff,stroke:#4a9eff
    style L1 fill:#4a2d1a,color:#fff,stroke:#d4845a
    style L3 fill:#2d4a2d,color:#fff,stroke:#6abf6a
```

**所有箭头指向 core，core 无任何指向外部的箭头。** 不是审查约定，是 Cargo 编译铁律——core 的 `Cargo.toml` 写了 `tokio` 或 `helix-im` 立即编译报错。


---

## §2 技术选型总表 + 为什么（第一性原理）

| 维度 | native | ffi | wasm | core/im | 为什么这样切 |
|---|---|---|---|---|---|
| Async 执行器 | tokio multi-thread | 宿主线程池 | spawn_local | 无执行器 | wasm 无 OS 线程，tokio 编不过 |
| Channel | tokio::sync | futures-channel | futures-channel | **futures-channel** | 唯一 executor-agnostic + 实现标准 Stream/Sink |
| async trait | async-trait(Send) | async-trait(Send) | async-trait(?Send) | `MaybeSend` cfg 切 | wasm `!Send`，AFIT 非 dyn-safe（见下） |
| Storage | sqlx + SQLite WAL | rusqlite + spawn_blocking | sqlite-wasm-rs + OPFS | `Storage` port | sqlx 依赖链 wasm 编不过（见下） |
| HTTP | reqwest 0.12 | 宿主回调 | gloo-net fetch | `HttpRequester` port | wasm fetch 无 streaming，IM v1 不受影响 |
| WebSocket | tokio-tungstenite | C ABI 回调 | web-sys WS + 手桥 | `Transport` port | 换库不改 core/im 一行 |
| 时钟 | SystemTime | host fn | js_sys::Date | `Clock` port 注入 | `Instant::now()` 在 wasm **运行时 panic** |
| panic | abort | 二选一：生产 abort；dev unwind 时 extern "C" 包 catch_unwind | abort | 返回 Result | panic 跨 FFI 是 UB；wasm 省 unwind 表 |

**两条最常被挑战的选型，根因图：**

```mermaid
flowchart LR
    SQLX["sqlx 0.8 (runtime-tokio)"] --> TOKIO[tokio] --> MIO[mio] --> EP["epoll_create1 / kqueue"]
    EP -.->|wasm32 无 OS syscall| FAIL["❌ cargo check --target wasm32 编译失败<br/>（切 async-std 同样失败）"]
    style FAIL fill:#5c1a1a,color:#fff
```

```mermaid
flowchart LR
    subgraph S1["为什么 Send 按 cfg 切"]
    A["native: tokio work-stealing<br/>要求 Future: Send + 'static"]
    B["wasm: spawn_local<br/>刻意不要 Send（web_sys WebSocket 持 JsValue 是 !Send）"]
    A & B --> C["MaybeSend：30 行 cfg trait 别名<br/>native=Send blanket / wasm=空 trait blanket<br/>配 cfg_attr(wasm32, async_trait 去 Send)"]
    end
```

> **为什么不用 AFIT 全静态分发省掉 Box**：AFIT 的关联 future 非 dyn-safe，而 `Box<dyn Transport>` 要同时持 native+wasm 两种实现，必须 dyn-safe；改泛型则 driver 类型被拉进 core，违反铁律。Box 开销不在 `step()` 热路径（step 同步、不调 port），可接受。

---

## §3 核心数据流：sans-IO 闭环（一张图）

```mermaid
flowchart TD
    IO["平台 I/O：WS 帧 / HTTP 响应 / Timer 到期 / 宿主命令"] -->|包成 Tick| STEP
    STEP["ExecutionShell.step(Tick, now_ms)<br/>严格同步 · 零 await/IO/spawn · 返回 &[Effect]"]
    STEP --> E1["Effect::Send → driver 写 WS 出站"]
    STEP --> E2["Effect::Persist{corr} → driver 写库(await)"]
    STEP --> E3["Effect::Http{corr} → driver 发请求"]
    STEP --> E4["Effect::ScheduleTimer → driver 注册定时器"]
    STEP --> E5["Effect::Emit(bytes) → EventSink（emit 入队立即返回 · 批次边界=一次 step）"]
    E2 -->|Tick::PortReply{corr,Ok/Err}| STEP
    E3 -->|Tick::PortReply| STEP
    E4 -->|Tick::Timer| STEP
    style STEP fill:#1a3a5c,color:#fff,stroke:#4a9eff
```

**确定性来源**：相同 Tick 序列 + 相同初态 = 相同 Effect 序列 → `proptest` 可回放（解 P3）。时间/随机/ID 全由 port 注入，core 不调 ambient `Instant::now`/`rand`。

---

## §4 四大能力 · 一个 port，三端流程图（headless 可插拔核心）

四能力 = 四个互不依赖的 trait，任一可单独 feature 启用。统一形态：**core 吐意图 → port → 三端 driver 各自兑现 → 结果回灌 Tick**。

### §4.1 EventBus —— `EventSink::emit(DomainEventBytes)`（同步 O(1)，core 只见不透明字节）

```mermaid
flowchart LR
    M["helix-im 吐 Effect::Emit(bytes)<br/>业务序列化在 ACL-1，core 不认识 ImDomainEvent"] --> P[["EventSink::emit(bytes)"]]
    P -->|native| N["tokio broadcast → Tauri app.emit('im:__bus__', envelope)<br/>→ Angular ImEventBusDispatcher 懒注册解复用"]
    P -->|wasm| W["postMessage(bytes) 批量 → 主线程 onmessage 派发"]
    P -->|ffi| F["C ABI fn(*const u8,len) → Dart SendPort Stream → UI 订阅"]
    style P fill:#1a3a5c,color:#fff
```
落地约束：emit 单点收口（CI grep gate 防回环）；fan-out 用 broadcast，core 内部 channel 用 futures-channel。

### §4.2 Storage —— `batch_upsert / monotonic_upsert / get`（热路径 O(1)，port 无 IM 语义）

```mermaid
flowchart LR
    M["Effect::Persist{corr, UpsertSpec{table:&'static str}}<br/>core 不知 'messages' / 'temporary_id' 是什么"] --> P[["Storage::batch_upsert / monotonic_upsert<br/>单 INSERT ON CONFLICT(PK) DO UPDATE · O(log N)≈O(1)<br/>禁 read-modify-write · monotonic 用 WHERE value>? MAX guard<br/>（通用单调写，cursor 只是 im 侧用途，core 不知道）"]]
    P -->|native| N["sqlx SqlitePool WAL · 三池对齐现网<br/>sync写 max=1 + 用户交互写 max=1 + 读 max=4（同一 WAL）<br/>编译期 SQL 检查"]
    P -->|wasm| W["sqlite-wasm-rs + OPFS<br/>仅 Worker 内 createSyncAccessHandle()"]
    P -->|ffi| F["rusqlite + spawn_blocking<br/>同步 C 调用不阻塞 tokio worker"]
    N --> R[["Tick::PortReply{corr, Ok/Err} 回灌 step"]]
    W --> R
    F --> R
    style P fill:#1a3a5c,color:#fff
```

### §4.3 HTTP —— `HttpRequester::request(HttpRequest)`（url 由 im 拼，重试不内置）

```mermaid
flowchart LR
    M["Effect::Http{corr, HttpRequest{url,method,body}}<br/>失败标 unSend，重试=业务语义留 helix-im"] --> P[["HttpRequester::request(req)"]]
    P -->|native| N["reqwest 0.12 · 连接池 / native-tls / streaming<br/>运行时 header 热注入(connectionId)"]
    P -->|wasm| W["gloo-net fetch · 无 streaming<br/>(IM v1 /sync/notify 是 req-resp，不受影响)"]
    P -->|ffi| F["宿主 HTTP 回调 / reqwest · futures-channel oneshot 包装"]
    N --> R[["Tick::PortReply{corr, status/body}"]]
    W --> R
    F --> R
    style P fill:#1a3a5c,color:#fff
```

### §4.4 WebSocket —— `Transport::connect/send/recv/close`（握手/重连/心跳=driver 私有，业务策略在 im）

```mermaid
flowchart LR
    C[["Transport::connect/send/recv/close<br/>core 只见四个纯语义操作"]]
    C -->|native| N["tokio-tungstenite .split()<br/>send/recv 分 task / TLS"]
    C -->|wasm| W["web-sys WebSocket + futures-channel 手桥(~80行)<br/>不依赖停滞的 ws_stream_wasm"]
    C -->|ffi| F["C ABI send_ws_frame + helix_on_ws_frame 注入帧"]
    N --> R[["Tick::Inbound(Bytes) → step"]]
    W --> R
    F --> R
    POLICY["重连退避 jitter / 25s 心跳半死检测 / cursor reset<br/>= helix-im 响应 Tick::Connected/Disconnected（core 之上）"] -.业务策略不进 driver.- C
    style C fill:#1a3a5c,color:#fff
```

**headless 可证**（现场跑）：`cargo tree -p helix-driver-native --features storage | grep tungstenite` → 空（只启 storage 不拉 ws）；`cargo test -p helix-core --no-default-features` → mock port 纯单测，不起 tokio/SQLite/WS。feature additive（Cargo unification 单调并集，互斥必爆）。

---

## §5 真实例子：发一条消息，三端怎么扭转

**统一 Core 时间线（三端一字不差，业务逻辑只写一次）**：

```
A 发送   step(Tick::Command{Send, tmp_9, content}) 吐:
           Persist{P1, Upsert(messages, key=tmp_9, Sending)}   ← 乐观落库
           Http{H1, POST /posts/create}                        ← 发服务端
           ScheduleTimer{T1, 15000ms}                          ← 超时
           Emit(Sending{tmp_9})                                ← UI 0 等待上屏
B 服务端  存库 → 分配 server_id + channel_event seq → WS 推 echo(带 tmp_9)
C 对账   step(Tick::Inbound{echo}) 吐:
           PersistFire{ ON CONFLICT key=tmp_9 → id=server_id, Sent }  ← 同行刷 id 无重排
           CancelTimer{T1}
           Emit(Received{完整消息})
收敛(C006)  echo 15s 没来 → Tick::Timer → 标 UnSend；迟到 echo 仍按 tmp_9 覆盖回 Sent
```

### ① PC = Tauri（driver-native · tokio 多线程）

```mermaid
sequenceDiagram
    autonumber
    participant UI as Angular UI (webview)
    participant C as helix-core (Shell+ImModule)
    participant D as driver-native (tokio)
    participant DB as SQLite (sqlx WAL)
    participant SV as 服务端
    UI->>C: Tick::Command{Send, tmp_9}
    C-->>D: [Persist P1, Http H1, Timer T1, Emit Sending]
    D-->>UI: app.emit(im:__bus__, Sending)  ← 0 等待上屏"发送中"
    par I/O 全异步并行，不挡 UI
        D->>DB: batch_upsert(tmp_9,Sending) 单事务 O(1) → PortReply P1
    and
        D->>SV: reqwest POST /posts/create → PortReply H1
    end
    SV-->>D: WS push echo{tmp_9, server_id, seq} (tungstenite)
    D->>C: Tick::Inbound{echo}
    C-->>D: [PersistFire ON_CONFLICT set id=server_id Sent, CancelTimer, Emit Received]
    D->>DB: upsert 刷 server_id O(1)
    D-->>UI: app.emit(im:__bus__, Received)  ← "已送达" 同行无重排
```

### ② Web = WASM（driver-wasm · 单线程 + Web Worker）

```mermaid
sequenceDiagram
    autonumber
    participant UI as Angular UI (主线程)
    participant W as Web Worker (core + wasm-driver)
    participant ST as OPFS SQLite (sqlite-wasm-rs)
    participant NET as fetch / web-sys WebSocket
    participant SV as 服务端
    UI->>W: postMessage(Command{tmp_9}) 批量过边界  ← 重活全在 Worker，主线程 60fps
    W->>W: core.step → [Persist, Http, Timer, Emit]
    W-->>UI: postMessage(Emit Sending) 批量回 ← 0 等待上屏
    par spawn_local 并发（非阻塞）
        W->>ST: OPFS 写 tmp_9 (Worker 内) → PortReply P1
    and
        W->>NET: fetch POST /posts/create → PortReply H1
    end
    SV-->>NET: WS push echo{tmp_9, server_id, seq}
    NET->>W: onmessage → Tick::Inbound{echo}
    W->>W: core.step → [PersistFire, CancelTimer, Emit Received]
    W->>ST: OPFS upsert 刷 server_id
    W-->>UI: postMessage(Emit Received) ← 气泡转"已送达"
```

### ③ 移动 = FFI（driver-ffi · .so 内置 tokio + Dart SendPort）

```mermaid
sequenceDiagram
    autonumber
    participant UI as Flutter UI (Dart)
    participant F as dart:ffi (C ABI)
    participant R as Rust .so (core + ffi-driver + tokio)
    participant DB as rusqlite (spawn_blocking)
    participant SP as Dart SendPort (Stream)
    participant SV as 服务端
    UI->>F: helix_send(ptr, bytes{tmp_9})
    F->>R: catch_unwind → Tick::Command{Send}
    R-->>SP: Emit Sending → Dart Stream ← FFI 即返回，UI 不阻塞
    par .so 内 tokio 多线程
        R->>DB: spawn_blocking rusqlite 写 tmp_9 → PortReply P1
    and
        R->>SV: reqwest POST /posts/create → PortReply H1
    end
    SV-->>R: WS push echo{tmp_9, server_id, seq}
    R->>R: Tick::Inbound{echo} → core.step
    R->>DB: upsert 刷 server_id O(1)
    R-->>SP: Emit Received → Dart Stream ← 气泡转"已送达"
    Note over F,R: opaque ptr 单 owner / panic 策略二选一：生产 cdylib panic=abort（无 unwind 可跨边界）；dev unwind 时 extern"C" 才包 catch_unwind
```

### 三端共因对照（同一份 core 逻辑，driver 各取所长）

| 维度 | PC/Tauri | Web/WASM | 移动/FFI | 结论 |
|---|---|---|---|---|
| UI 被 I/O 阻塞 | 否(tokio 线程) | 否(Worker 隔离) | 否(SendPort 流) | **渲染永不碰 I/O** |
| 乐观上屏 | app.emit 立回 | postMessage 立回 | SendPort 立回 | **0 等待先上屏** |
| 并发 | tokio 多线程 | spawn_local | .so 内 tokio | core 不假设线程 |
| 跨边界 | 进程内 | 批量过 wasm↔JS | 批量过 FFI | **批处理不逐条** |
| 热路径写 | sqlx O(1) | OPFS O(1) | rusqlite O(1) | **单 INSERT/UPDATE WHERE PK** |
| 对账/超时/cursor | 同一份 | 同一份 | 同一份 | **写一次、可单测、三端一致** |

---

## §6 一致性不变量（红线：不一致 = 架构失败）

| 不变量 | 守护机制 |
|---|---|
| 对账幂等 | temporaryId 为 PK + ON CONFLICT 刷 server_id，echo/timer 两序收敛 |
| Emit 非阻塞 · 乐观上屏 | EventSink::emit 三端契约：入队立即返回（不阻塞 step）；UI 实时性靠 Emit 事件驱动，不依赖「写后立读可见」；Persist/Http 任意到达顺序靠幂等 upsert + 单调 cursor 收敛 |
| 无自动补发 | Module::handle 不产出自动重发 Effect，UnSend 是终态 |

> **§0 的 P2「Bug 修三处、行为分叉」在此架构下不可能发生**——从来只有一份 cursor 逻辑在 helix-im，三端 driver 执行同一 spec。

---

## §7 路线图（核心问题还要几个 MVP）+ 决策请求

| 阶段 | 内容 | 工期 |
|---|---|---|
| M0 | 骨架三栖绿 + 7 护栏 + core 零运行时实测 |  |
| M1 | core 单测补全，sans-IO 可回放 |  |
| **M2** | **native 端到端跑通完整 IM（发送/对账/同步/心跳）** |  |
| M3 | wasm OPFS + WS 联调，浏览器演示 IM |  |
| M4 | FFI 移动端联调 | |
| M5 | presence 挂载（register 一行，**core 一行不改**）= 框架 vs 补丁终极验收 | |

**请求批准 M2**：1 名 Rust，3-4 周；交付 Tauri Desktop 完整 IM 路径运行 + 三栖编译持续绿；验收 `cargo test -p helix-im` 通过 + §10 验证命令（cargo tree 零运行时 / core 零 IM 概念 grep）现场为真。

---



## §10 回到那句反问

> 「我现在做的，是为原点构建的框架，还是在打各种补丁？」

- **headless 守住**：core 里 `channel/message/temporaryId` = 0 行（实测），等价「view 里没有 style」。
- **语义层建出来**：EventBus/Storage/HTTP/WS 是四个真实 port trait + 三端实现（§4 四张图），不是「该建的没人实现」。
- **没反复横跳**：关键决策落进 `decisions.md` ADR + 7 护栏，每条有第一性原理背书。
- **框架不是补丁**：每个选型对应 P1/P2/P3（§2/§4/§5）；M5 presence 挂载零改 core 用代码裁决。

**守原点不漂移的机制 = Cargo 依赖图 + 7 张可执行护栏 + 三端同一份 Tick/Effect 时间线。
