# Helix ↔ Crux 参照对照（边界三分法 + 还能从 Crux 取什么）

> 2026-06-15 · Crux = helix 的参照实现（Red Badger，`/workspace/rustWorkspace/crux`，源码已核 `crux_core` 0.19）。
> 本文回答两件事：① **Crux 的能力边界到底在哪**（做到 / 故意不做 / 真缺口，别误判）；② **除已采用的，Crux 还有哪些方向值得参考**。
> 构件映射 / helix⊄crux 分歧 / 性能缺口的**详表**见 [`../ROADMAP-full.md` §4/§5/§6](../ROADMAP-full.md)（本文不重复，只给结论 + 新增内容）；设计理念见 [`helix-core-im-架构设计.md` §7.0](./helix-core-im-架构设计.md)；方向A性能方案见 [`helix-core-view-方向A设计.md` §4.1](./helix-core-view-方向A设计.md)。

---

## §1 边界三分法（最容易误判，钉死）

| 类别 | 内容 | 对 helix 的含义 |
|---|---|---|
| **✅ Crux 做到了（要抄）** | headless Rust 核 + 多端共享一份（iOS Swift / Android Kotlin / Web TS）· 多端 IPC 边界（`Bridge` bincode + `EffectId` Slab 配对 + `typegen`）· ViewModel 投影（`Core::view()`）· Render 通知 | helix 今天 = Crux 去掉 view()/ViewModel/Render/typegen 四件；Phase 6 照补 |
| **🚫 Crux 故意不做（headless 定义，非缺陷）** | **渲染**（画像素 / 出 DOM）——永远归各端原生 UI（SwiftUI/Compose/React）。core 只出**数据形态** ViewModel（「未读=3」「这段是 mention」），不出像素 | 「用 Rust 画 UI」= Dioxus 路 = helix **方向 B 红线**，两家都拒。不是「Crux 没做到渲染」，是渲染本不该 core 做 |
| **⚠️ Crux 真缺口（唯一，且正好咬海量消息）** | `Core::view()` 每次 Render **整份** ship ViewModel，crux_core **零 diff/patch/windowing**；官方做法 = shell 自己虚拟滚动。Crux 实战样例都是小 app（counter/notes），未针对海量高频 IM 打磨 | chat 几百会话/几千消息/每条触发一次 → 整份 ship 扛不住。**大列表增量/窗口化投影 = helix 在 Crux 之上自研**（[方向A §4.1](./helix-core-view-方向A设计.md)：A 全量 + B 窗口化 + C diff + D 分块 + E 脏标记），单列预算，非抄 |

**一句话**：Crux 做到了「headless Rust 核 + 多端 + IPC + 投影」；渲染它故意不做（归原生 UI）；唯一没给的是「**海量列表的增量/窗口化数据供给**」——这正是 helix Phase 7 自研那一层。

---

## §2 还能从 Crux 取什么（已采用之外的参考方向）

> 已采用 = App/view/Render/Bridge/typegen/EffectId↔corr（见 ROADMAP-full §4）。以下是**额外**值得借鉴的，按 helix 优先级排。

| # | Crux 方向 | 源码位置 | helix 怎么用 | 优先级 |
|---|---|---|---|---|
| 1 | **测试范式：断言 Effect 不执行** —— `AppTester` 喂 event → 拿回 effects 列表直接断言（不跑 I/O），resolve 注入结果继续 | `crux_core` test utils + 各 example 的 `*_test.rs` | 直接强化 helix 的 replay/proptest：喂 `Tick` 序列 → 断言 `&[Effect]`，注入 `PortReply` 续跑——**与 helix sans-IO 天然契合，几乎照搬** | **高** |
| 2 | **typegen 双后端** —— serde-reflection（稳）/ Facet（新，编译期反射） | `crux_core/src/type_generation/{serde,facet}.rs` | Phase 6 出 ViewModel 的 TS interface 时的选型：先 serde-reflection 稳妥，关注 Facet 演进 | **高** |
| 3 | **Command 组合子** —— `Command::all`（并发）/ `and`（串行，前出喂后入）/ `then_send`（链式回 Event）/ `notify_shell`（fire-forget）vs `request_from_shell`（要回执） | `crux_core/src/command/mod.rs` | helix **不抄 executor**，但抄**语义**：`notify_shell`↔`PersistFire` / `request_from_shell`↔`Persist{corr}`；多 Effect 的并发/串行/链式编排,作为 helix-im 写 peek→persist→commit 跨 step 流程的参照 | 中 |
| 4 | **Bridge 可配置序列化** —— `Bridge<A, BincodeFfiFormat>` 默认 / `JsonFfiFormat` 可换 | `crux_core/src/bridge/mod.rs` | helix 边界 dev 期用 JSON（可读、好调试），生产用 bincode（紧凑）——同一边界两种格式可切 | 中 |
| 5 | **Capability authoring 模式** —— 写一个新能力 = 定义 `Operation`（含 `Output` + `register_types`）+ 一个返回 `Command` 的构造器 | `crux_http` / `crux_kv` / `crux_time` | helix「加新 Effect 变体 / 加新 port / 第二模块要新 I/O 形态」的范本：照 Operation 的最小契约扩 port | 中 |
| 6 | **examples 的 shell 三责接线** —— forward event / execute effect+resolve / pull view，每端一份薄壳 | `crux/examples/{counter,notes,cat_facts}/` | helix 三端 driver（Tauri/wasm/ffi）接线的对照样板，尤其 FFI shell 的 update/resolve/view 三方法签名（Phase 5） | 中 |
| 7 | **`#[effect]` 宏 codegen 范式** —— 把 `enum{Render, Http(..)}` 自动展开成 `Request<Op>` 包裹 + `EffectFfi` 变体 + `Export` impl | `crux_macros` | helix 拍板**单一手写 enum**，暂不需要；但若将来要 per-capability typegen 自动化，这是范本（保留观察，不现在做） | 低 |
| 8 | **Crux Book（方法论）** —— sans-IO / capability / shell 模式的成文方法论 | crux 官方 mdbook | 团队对齐读物；helix 的 GOAL/架构设计已吸收其精神，按需回查 | 低 |

---

## §3 取舍铁律（防带偏）

- **抄形状，不抄执行器**：抄 Crux 的沙漏形状 + Render/Bridge/typegen 三件套 + EffectId↔corr 配对；**不抄** `Command` async executor——helix core 内零 executor，比 Crux 更纯 headless（[ROADMAP-full §5](../ROADMAP-full.md)）。
- **渲染线两家一致**：core 出数据/切片/补丁，UI 出窗口/像素；任何方案都不许 core 碰像素 = 不退方向 B。
- **海量供给是自研**：Crux 整份 ship 的模型对千条消息不够，§1 的真缺口由 helix Phase 7 §4.1 自补，不能假设「抄 Crux 就有」。

---

> 交叉引用：完整 roadmap + 构件映射 [`../ROADMAP-full.md`](../ROADMAP-full.md) · 设计理念 [`helix-core-im-架构设计.md` §7.0](./helix-core-im-架构设计.md) · 方向A性能 [`helix-core-view-方向A设计.md` §4.1](./helix-core-view-方向A设计.md) · 北极星边界 [`../GOAL.md` §6](../GOAL.md) · 文档地图 [`../README.md`](../README.md)。
