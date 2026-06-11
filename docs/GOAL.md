# Helix — 项目北极星 / L2 目标常量（设计理念不变量，不可偏离）

> 这是 Helix 的**最高层目标与设计理念锚点**（harness 母本四件套之 GOAL.md，L2 常量）。
> 任何改动若与本文 §1 设计理念不变量 / §2 硬约束冲突 → **先停下问，不偏离**。
> 每次会话开局必读（加载顺序：~/.claude/CLAUDE.md → 本项目 CLAUDE.md → **docs/GOAL.md** → SESSION → harness/README）。

---

## §0 一段话定义（北极星，背下来）

**Helix 是我们的「运行时无关确定性业务内核」**——目标是一份业务逻辑只写一次，就能编译到三端（native/Tauri、FFI/移动、WASM/浏览器）且行为一致、可回放可测试，IM 只是跑在它上面的第一个业务模块，内核本身不认识任何业务；核心设计理念是 **sans-IO 六边形**：`helix-core` 零运行时、零平台依赖（实测只依赖 serde/thiserror/futures-channel，三栖编译绿），它不做 I/O、不持有 tokio，只定义一组**端口（ports）**和一个纯状态机执行壳——喂入（入站帧 / now_ms / 端口响应）、吐出（Effect），真正的网络、存储、线程、时间、随机、ID 全部由各平台的薄驱动注入（这就是"确定性"的来源，也是并发不进 core 的原因），依赖方向严格单向 `driver → core` 永不反向；四大能力都以这套 ports 表达——**EventBus/事件通信**走 `EventSink` 端口，core 通过 `emit(DomainEvent)` 单点收口把强类型领域事件吐给宿主，内部用运行时中立的 futures-channel，且 Effect 跨 wasm/FFI 边界**批量**传不逐条（借 Dioxus sledgehammer 经验）；**Storage** 走 `Storage` 端口，用通用 `Upsert/Get/CursorUpsert` spec 表达而不泄漏业务 schema，native 后端 sqlx+SQLite、wasm 后端 sqlite-wasm-rs+OPFS（仅 Worker 内）、移动端 rusqlite，热路径强制 O(1)（单 INSERT/UPDATE WHERE PK、`&'static str` 不堆分配、禁 read-modify-write）；**HTTP 与 WebSocket Transport** 各走 `HttpClient` / `Transport` 端口、一个 trait 按 cfg 选实现，native 用 tokio-tungstenite + reqwest（可上多线程，正是"原生→多线程"），浏览器用 web-sys WebSocket + fetch（单线程 spawn_local），移动端走 host 回调——同一份 core 逻辑骑在三种平台运行时上，各取所长、互不污染，这一切再由 7 张 HX-C harness 卡（零运行时依赖 / Send 边界 / wasm CI / features additive / 热路径零分配 / FFI 铁律 / 边界批处理）编译成可执行护栏守住。

---

## §1 设计理念不变量（违反任一 = 偏离北极星，必须停下）

1. **运行时无关**：只有 `helix-core` 编到三端，所以**只有它**必须中立（零 tokio/sqlx/web-sys/tauri，用 futures-channel）。→ `HX-C001`
2. **sans-IO / 六边形**：core 不做 I/O、不持运行时，纯状态机「喂入 → 吐 Effect」，I/O 由驱动做。
3. **确定性**：时间 / 随机 / ID 由 host 注入，core 不调 ambient `Instant::now`/`rand`/生成 id；同输入同输出可回放。
4. **并发不进 core**：core 单线程可跑；多线程是 native 驱动的事（wasm 单线程）。Send 按 cfg 切，默认 Unsync。→ `HX-C002`
5. **依赖单向**：`driver → core`，永不反向；core 只定义 ports，驱动在边缘实现。
6. **业务不污染内核**：DomainEvent / wire 名 / temporaryId / 业务 schema 留在模块，core 只有通用 spec。IM 是首个 guest，不是 core 的一部分。
7. **能力即胶囊**：EventBus/Storage/HTTP/WS 各自独立、feature 可加、互不强依赖。→ `HX-C004`

---

## §2 硬约束（绝不做）

- ❌ 不在 `helix-core` import 任何具体运行时/平台 crate（tokio/sqlx/reqwest/web-sys/tauri）
- ❌ 不把并发模型/线程假设写进 core（不假设 multi-thread）
- ❌ 不用 `&'a` 生命周期载荷穿 async/channel（用 owned/Copy/arena id）→ `HX-C002`
- ❌ 不在热路径堆分配 / read-modify-write（O(1) only）→ `HX-C005`
- ❌ 不让 panic unwind 跨 FFI；opaque ptr 不双 own → `HX-C006`
- ❌ 不逐条过 wasm/FFI 边界传 Effect（批量）→ `HX-C007`
- ❌ 不设互斥 feature → `HX-C004`
- ❌ 不让业务 schema 泄漏进 core 的 ports
- ❌ 不为了"通用"预建用不到的插件框架（通用性靠 ≥1 真实模块 + 第二模块插得进挣）

---

## §3 成功标准（measurable）

- ✅ `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 输出为空
- ✅ 三栖编译全绿：`cargo check` native（core/im/driver-native/driver-ffi）+ `--target wasm32-unknown-unknown`（core/driver-wasm）
- ✅ `grep channel/message/temporaryId crates/helix-core` = 0（纯 IM 概念隔离；cursor 见 ADR-004 待定）
- ✅ 加新业务模块 = 实现 `Module` trait + `register()`，**core 一行不改**（开闭原则）
- ✅ 加新平台 = 实现 ports，**core/模块一行不改**
- ✅ 同一份 core 在集成测试（无 webview）/ Tauri / 浏览器 / 移动端零改复用

---

## §4 反模式（已否定，防重提）

| 否定的方案 | 为什么否 |
|---|---|
| 把 tokio/sqlx/tungstenite bake 进 core（初版 im_core）| 单平台假设，wasm 立刻碎；已拆三层 |
| 单平台 YAGNI 砍掉 Storage/Clock/Transport 抽象 | 三栖目标下那三个是命根子；YAGNI 边界必须在目标平台集合下画 |
| core 强加 Send bound | wasm `!Send` future 冲突；默认 Unsync + 跨线程 opt-in Sync |
| Effect 逐条过边界 | 固定开销线性叠加；Dioxus sledgehammer 批处理 |
| 互斥 feature（native/wasm 二选一）| feature unification 单调性必爆 |

---

## §5 不偏离机制

- **本文** = L2 常量，每次会话开局必读（设计理念锚点）。
- **CLAUDE.md 顶部北极星指针** → 每次必加载，指回本文。
- **7 张 HX-C harness 卡** = 把 §1/§2 编译成 grep/CI/正反对照的**可执行护栏**（docs/harness/）。
- **decisions.md** = 已拍板的 ADR（防反复推导）。
- 改动与本文冲突 → 停下问；新决策 → 追加本文 §4 或 decisions.md。
