# Helix — 项目北极星 / L2 目标常量（设计理念不变量，不可偏离）

> 这是 Helix 的**最高层目标与设计理念锚点**（harness 母本四件套之 GOAL.md，L2 常量）。
> 任何改动若与本文 §1 设计理念不变量 / §2 硬约束冲突 → **先停下问，不偏离**。
> 每次会话开局必读（加载顺序：本项目 AGENTS.md → **docs/GOAL.md** → SESSION → harness/README）。

---

## §0 一段话定义（北极星，背下来）

**Helix 是我们的「运行时无关确定性业务与交互内核」**——目标是一份业务逻辑和一份跨端 UI 交互语义只写一次，产品目标集合为 **PC 端 Tauri（native）+ 手机端 Flutter（FFI）+ Web**；当前交付事实是 native/FFI 已产品化，Web 的 Rust 平台壳、最小浏览器 glue 与正式 gate 已落地，但真浏览器→真 Go 产品链仍待环境验收。IM 是首个业务模块，`helix-core` 不认识任何业务或具体 UI；同一份 `helix-im` 负责三端共同的业务门面、local-first/remote fallback、WS 协议生命周期，以及 `SurfaceModel / ViewNode / ActionBinding / UiDirective` 语义交互协议。PC/Flutter 复用完整 SQLite/HTTP/WS；Web 使用 Rust 有界 session store 与 Browser Fetch，物理 WebSocket 生命周期由浏览器适配器兑现。历史 `helix-driver-wasm` 已删除。

**本项目对 Headless 的定义**：Headless 不是“只能输出扁平数据 ViewModel”，也不是“Rust 负责画具体 UI”。它表示 Helix 可以拥有完整的、平台无关的**呈现状态机与交互图**，包括语义节点、有序关系、动作地址和一次性 UI 语义；但它不依赖任何 renderer、窗口系统、组件框架或 I/O runtime。ViewModel 只是该合同里的可重放呈现状态，不能替代动作绑定、directive、ack/resync 和上行事实协议。

---

## §1 设计理念不变量（违反任一 = 偏离北极星，必须停下）

1. **运行时无关**：`helix-core` 必须能被 PC native、Flutter FFI 与 Web WASM 共用，因此保持零 tokio/sqlx/reqwest/web-sys/tauri；业务模块同样只依赖 core ports，不直接依赖平台 runtime。→ `HX-C001`
2. **sans-IO / 六边形**：core 不做 I/O、不持运行时，纯状态机「喂入 → 吐 Effect」，I/O 由驱动做。
3. **确定性**：时间 / 随机 / ID 由 host 注入，core 不调 ambient `Instant::now`/`rand`/生成 id；同输入同输出可回放。
4. **并发不进 core**：core 单线程可跑；多线程是 native/FFI 驱动的事。Send 按 cfg 切，默认 Unsync。→ `HX-C002`
5. **依赖单向**：`driver → core`，永不反向；core 只定义 ports，驱动在边缘实现。
6. **业务不污染内核**：DomainEvent / wire 名 / temporaryId / 业务 schema 留在模块，core 只有通用 spec。IM 是首个 guest，不是 core 的一部分。
7. **能力即胶囊**：EventBus/Storage/HTTP/WS 各自独立、feature 可加、互不强依赖。→ `HX-C004`
8. **交互语义一次实现**：`helix-im` 可以产出平台无关的语义 view tree、稳定动作地址和一次性 UI directive；各端不得再实现消息对账、未读、窗口、权限或导航 fallback 等业务交互决策。具体 DOM、Angular Component、Flutter Widget、CSS、像素和动画仍属于平台 shell。→ `ADR-015` / `HX-C018`

---

## §2 硬约束（绝不做）

- ❌ 不在 `helix-core` import 任何具体运行时/平台 crate（tokio/sqlx/reqwest/web-sys/tauri）
- ❌ 不把并发模型/线程假设写进 core（不假设 multi-thread）
- ❌ 不用 `&'a` 生命周期载荷穿 async/channel（用 owned/Copy/arena id）→ `HX-C002`
- ❌ 不在热路径堆分配 / read-modify-write（O(1) only）→ `HX-C005`
- ❌ 不让 panic unwind 跨 FFI；opaque ptr 不双 own → `HX-C006`
- ❌ 不逐条过 Web/FFI 边界传 Effect/Event（按 step/batch）→ `HX-C007`
- ❌ 不设互斥 feature → `HX-C004`
- ❌ 不让业务 schema 泄漏进 core 的 ports
- ❌ 不让 `helix-core` 认识 `ViewNode`、`ActionBinding`、`UiDirective` 或任何 IM/UI schema；这些只属于业务模块
- ❌ 不在 `helix-im` 输出 DOM tag、Angular Component、Flutter Widget、CSS selector、像素、动画时长或平台句柄
- ❌ 不为了"通用"预建用不到的插件框架（通用性靠 ≥1 真实模块 + 第二模块插得进挣）

---

## §3 成功标准（measurable）

- ✅ `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 输出为空
- ✅ 已产品化双端编译全绿；A4 `helix-driver-web` 已通过 `wasm32-unknown-unknown` 构建，正式 Web CI/HX-C003 已在 A5 激活为 native + wasm final artifact 双目标门
- ✅ `grep channel/message/temporaryId crates/helix-core` = 0（纯 IM 概念隔离；cursor 概念归 helix-im，core 只有通用 `monotonic_upsert` 单调原语——ADR-004 已闭环，见 [decisions.md](./harness/decisions.md#adr-004) / 纪要拍板 #1）
- ✅ 加新业务模块 = 实现 `Module` trait + `register()`，**core 一行不改**（开闭原则）
- ✅ 加新平台 = 实现 ports，**core/模块一行不改**
- ✅ 同一份 core/im 在集成测试（无 webview）/ Tauri / Flutter FFI / Web Rust 壳零改复用；同 Tick native/FFI/Web 业务输出一致已由 A4 parity 测试证明，真实 Chromium + WASM 已验证 hello/local-first/HTTP fallback/文本 WS，真 Go 完整链路仍待环境验收
- ✅ 三端共享 `helix-im::client_api` 与异步 query 主路径；driver 不保留 query/HTTP fallback 业务旁路
- ✅ 三端共享业务语义但允许平台能力不同：PC/Flutter 完整 SQLite；Web 使用不跨刷新的有界内存 session store
- ✅ core WS port 只表达 outbound frame；Web WS 帧处理复用 `helix-im`，真实 socket、浏览器生命周期与鉴权由 Web 平台适配器兑现
- ✅ 同一 `UiEvent/HostObservation/EffectResult/WS` 剧本在 native/FFI/Web 产生字节等价 `InteractionFrame`；各端只使用生成合同、静态 renderer registry 和平台 executor

---

## §4 反模式（已否定，防重提）

| 否定的方案 | 为什么否 |
|---|---|
| 把 tokio/sqlx/tungstenite bake 进 core（初版 im_core）| 单平台假设，wasm 立刻碎；已拆三层 |
| 单平台 YAGNI 砍掉 Storage/Clock/FrameSender 抽象 | PC + Flutter + Web 目标下这些能力边界仍是命根子；YAGNI 边界必须在目标平台集合下画 |
| core 强加 Send bound | 会把单线程/FFI 宿主形态锁死；默认 Unsync + 跨线程 opt-in Sync |
| Effect 逐条过边界 | 固定开销线性叠加；Dioxus sledgehammer 批处理 |
| 互斥 feature（native/wasm 二选一）| feature unification 单调性必爆 |
| 复活已删除的历史 `helix-driver-wasm` | 正式 Web 边界已经由 `helix-driver-web` 兑现；旧骨架不得恢复为第二套实现 |
| 为追求“三端一致”强迫 Web 复制完整 SQLite | 共享的是协议与业务语义，不是持久化数据量；Web 可选择无持久化或受限缓存 |
| Web 只持久化 cursor、不持久化其声明已应用的数据 | 刷新后 cursor 超前会永久跳过缺失投影；必须联合记录 cache coverage/window 或冷启动重同步 |
| 每端各写一套 ProjectionStore/BLoC/QuickJS 业务 reducer | temporaryId 对账、窗口排序、未读和权限会三端漂移；统一进 `helix-im` 的 InteractionProtocol |
| 把语义 `render_kind` 直接绑定具体 Component/Widget 类名 | 业务模块会反向依赖平台实现；`render_kind` 只能是稳定、平台无关的语义 renderer key |

---

## §5 不偏离机制

- **本文** = L2 常量，每次会话开局必读（设计理念锚点）。
- **AGENTS.md 顶部北极星指针** → 每次必加载，指回本文。
- **18 张 HX-C harness 卡** = 把 §1/§2 编译成 grep/CI/正反对照的**可执行护栏**（docs/harness/）。
- **decisions.md** = 已拍板的 ADR（防反复推导）。
- 改动与本文冲突 → 停下问；新决策 → 追加本文 §4 或 decisions.md。

---

## §6 Headless 抽象边界（语义交互属于内核 / 具体 UI 与 I/O 属于平台）

> 2026-07-15 用户拍板：旧“方向 A 只能吐扁平数据 ViewModel”被 ADR-015 覆盖，不再作为现行设计入口。
> 新边界的真源是 [`2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`](./specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md)：统一的是跨端**语义交互合同**，不是具体 UI 工具包。

| 边界层 | 含义 | 结论 |
|---|---|---|
| **Headless 交互内核（目标）** | `helix-im` 产出 `InteractionFrame(ViewMutation + ActionBinding + UiDirective)`：语义 view tree、稳定顺序、opaque action、focus/reveal/reset/dismiss/fallback 等跨端语义 | ✅ 多端统一实现；平台机械 apply、执行和 ack |
| **具体平台 UI（边界外）** | Rust 输出 DOM tag、Angular Component、Flutter Widget、CSS selector、像素、动画或平台对象生命周期 | ❌ 反向依赖平台，破坏 runtime-agnostic 与可替换 renderer |
| **真实 I/O（边界外）** | core 自己执行网络、存储、timer、文件或线程调度 | ❌ 违反 §1.2 / §2（sans-IO）；多端复用立刻碎，`HX-C001` 守门 |

**Headless 交互不变量**：
- `SurfaceModel / ViewNode / ActionBinding / UiDirective` 是 IM 业务交互类型，只能在 `helix-im`；`helix-core` 只搬运不透明批次并提供确定性执行机制。
- `ViewNode.render_kind` 是语义 renderer key（如 `message.row.v1`），不得出现具体组件、Widget、DOM 或样式名；各端通过静态 registry 映射自己的组件。
- `ActionBinding.action_ref` 只暴露 opaque、surface-scoped 动作地址；权限、command 选择和 payload 组装留在 `helix-im`。
- `UiDirective` 只表达跨端语义“做什么”，平台决定“怎么做”；持久业务结果必须同时进入 ViewModel，不能只靠一次性 directive 成为权威。
- frame 必须 revision/epoch 可校验、确定性可回放、跨 FFI 批量传输；同一领域事实引起的多节点变化原子 apply。→ `HX-C007`
- shell 可以维护像素滚动、动画、原生焦点句柄和可丢弃 frame cache，但不得恢复业务 reducer、temporaryId 对账、read candidate、窗口排序或权限分支。

**判据一句话**：凡是“多个平台应得出同一业务呈现与交互结果”的规则，归 `helix-im`；凡是“这个平台具体如何画、如何动、如何取得句柄或执行 I/O”的实现，归平台 shell/driver。只输出 ViewModel 会漏掉前一类中的动作与交互时序，因此不是本项目 Headless 的终点。

**时序硬约束**：InteractionProtocol 在基础 DomainModel、Persist barrier 与收发/同步链路之上增量实现；不得用纯文档协议掩盖当前 `declared/blocked` Gate，也不得让 UI 暂时代偿尚未进入 Helix 的业务规则。
