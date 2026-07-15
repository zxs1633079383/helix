# Helix — 项目北极星 / L2 目标常量（设计理念不变量，不可偏离）

> 这是 Helix 的**最高层目标与设计理念锚点**（harness 母本四件套之 GOAL.md，L2 常量）。
> 任何改动若与本文 §1 设计理念不变量 / §2 硬约束冲突 → **先停下问，不偏离**。
> 每次会话开局必读（加载顺序：本项目 AGENTS.md → **docs/GOAL.md** → SESSION → harness/README）。

---

## §0 一段话定义（北极星，背下来）

**Helix 是我们的「运行时无关确定性业务与交互内核」**——目标是一份业务逻辑和一份跨端 UI 交互语义只写一次，产品目标集合为 **PC 端 Tauri（native）+ 手机端 Flutter（FFI）+ Web**；当前交付事实是 native/FFI 已产品化、Web 自 2026-07-13 按 ADR-014 进入 W0 可达性与契约验证，尚未交付。IM 只是跑在它上面的第一个业务模块，`helix-core` 本身不认识任何业务或具体 UI；核心设计理念是 **sans-IO 六边形**：`helix-core` 零运行时、零平台依赖，它不做 I/O、不持有 tokio，只定义端口和纯状态机执行壳。`helix-im` 负责三端共同的 WS 帧解析、sequence/cursor、echo、已读、补偿同步，以及 `SurfaceModel / ViewNode / ActionBinding / UiDirective` 组成的语义交互协议；各端只兑现 I/O、平台事实和具体 DOM/Component/Widget 渲染。时间、随机、ID 与真实 I/O 全由薄驱动注入，依赖方向严格单向 `driver → core` 永不反向。PC native 与 Flutter FFI 继续共同复用 `helix-driver-host` 的完整 SQLite/HTTP/WS；Web 规划由 `helix-driver-web` 在 Dedicated Worker 中兑现浏览器 Transport 和 session store，并只把近期消息、会话摘要、草稿或 UI 偏好投影为有界可淘汰缓存。现有 `helix-driver-wasm` 仍是历史实验，不等于正式 Web 产品壳。

---

## §1 设计理念不变量（违反任一 = 偏离北极星，必须停下）

1. **运行时无关**：`helix-core` 必须能被 PC native、Flutter FFI 与 Web WASM 共用，因此保持零 tokio/sqlx/reqwest/web-sys/tauri；业务模块同样只依赖 core ports，不直接依赖平台 runtime。→ `HX-C001`
2. **sans-IO / 六边形**：core 不做 I/O、不持运行时，纯状态机「喂入 → 吐 Effect」，I/O 由驱动做。
3. **确定性**：时间 / 随机 / ID 由 host 注入，core 不调 ambient `Instant::now`/`rand`/生成 id；同输入同输出可回放。
4. **并发不进 core**：core 单线程可跑；多线程是 native/FFI 驱动的事。Send 按 cfg 切，默认 Unsync。→ `HX-C002`
5. **依赖单向**：`driver → core`，永不反向；core 只定义 ports，驱动在边缘实现。
6. **业务不污染内核**：DomainEvent / wire 名 / temporaryId / 业务 schema 留在模块，core 只有通用 spec。IM 是首个 guest，不是 core 的一部分。
7. **能力即胶囊**：EventBus/Storage/HTTP/WS 各自独立、feature 可加、互不强依赖。→ `HX-C004`
8. **交互语义一次实现**：`helix-im` 可以产出平台无关的语义 view tree、稳定动作地址和一次性 UI directive；各端不得再实现消息对账、未读、窗口、权限或导航 fallback 等业务交互决策。具体 DOM、Angular Component、Flutter Widget、CSS、像素和动画仍属于平台 shell。→ `ADR-015`

---

## §2 硬约束（绝不做）

- ❌ 不在 `helix-core` import 任何具体运行时/平台 crate（tokio/sqlx/reqwest/web-sys/tauri）
- ❌ 不把并发模型/线程假设写进 core（不假设 multi-thread）
- ❌ 不用 `&'a` 生命周期载荷穿 async/channel（用 owned/Copy/arena id）→ `HX-C002`
- ❌ 不在热路径堆分配 / read-modify-write（O(1) only）→ `HX-C005`
- ❌ 不让 panic unwind 跨 FFI；opaque ptr 不双 own → `HX-C006`
- ❌ 不逐条过 FFI 边界传 Effect（批量）→ `HX-C007`
- ❌ 不设互斥 feature → `HX-C004`
- ❌ 不让业务 schema 泄漏进 core 的 ports
- ❌ 不让 `helix-core` 认识 `ViewNode`、`ActionBinding`、`UiDirective` 或任何 IM/UI schema；这些只属于业务模块
- ❌ 不在 `helix-im` 输出 DOM tag、Angular Component、Flutter Widget、CSS selector、像素、动画时长或平台句柄
- ❌ 不为了"通用"预建用不到的插件框架（通用性靠 ≥1 真实模块 + 第二模块插得进挣）

---

## §3 成功标准（measurable）

- ✅ `cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'` 输出为空
- ✅ 已交付双端编译全绿：`cargo check` native（core/im/driver-native/driver-ffi）；Web 在 W1 建立正式壳后追加 wasm target gate
- ✅ `grep channel/message/temporaryId crates/helix-core` = 0（纯 IM 概念隔离；cursor 概念归 helix-im，core 只有通用 `monotonic_upsert` 单调原语——ADR-004 已闭环，见 [decisions.md](./harness/decisions.md#adr-004) / 纪要拍板 #1）
- ✅ 加新业务模块 = 实现 `Module` trait + `register()`，**core 一行不改**（开闭原则）
- ✅ 加新平台 = 实现 ports，**core/模块一行不改**
- ✅ 同一份 core/im 在集成测试（无 webview）/ Tauri / Flutter FFI 零改复用；Web W1 验收要求同 Tick 剧本输出同业务 Effect/DomainEvent
- ✅ 三端共享业务语义但允许平台能力不同：PC/Flutter 完整 SQLite；Web session state + bounded cache，缓存不是权威消息库
- ✅ Web WS 帧处理复用 `helix-im`；真实 socket、浏览器生命周期与鉴权由 `helix-driver-web` 兑现
- ✅ 同一 `UiEvent/HostObservation/EffectResult/WS` 剧本在 native/FFI/Web 产生字节等价 `InteractionFrame`；各端只使用生成合同、静态 renderer registry 和平台 executor

---

## §4 反模式（已否定，防重提）

| 否定的方案 | 为什么否 |
|---|---|
| 把 tokio/sqlx/tungstenite bake 进 core（初版 im_core）| 单平台假设，wasm 立刻碎；已拆三层 |
| 单平台 YAGNI 砍掉 Storage/Clock/Transport 抽象 | PC + Flutter + Web 目标下那三个仍是命根子；YAGNI 边界必须在目标平台集合下画 |
| core 强加 Send bound | 会把单线程/FFI 宿主形态锁死；默认 Unsync + 跨线程 opt-in Sync |
| Effect 逐条过边界 | 固定开销线性叠加；Dioxus sledgehammer 批处理 |
| 互斥 feature（native/wasm 二选一）| feature unification 单调性必爆 |
| 把历史 `helix-driver-wasm` 直接当正式 Web 实现 | Web 已按 ADR-014 重启，但历史骨架仍有大量未兑现端口；正式壳先过 W0/W1，再命名为 `helix-driver-web` |
| 为追求“三端一致”强迫 Web 复制完整 SQLite | 共享的是协议与业务语义，不是持久化数据量；Web 应使用 session state + bounded cache |
| Web 只持久化 cursor、不持久化其声明已应用的数据 | 刷新后 cursor 超前会永久跳过缺失投影；必须联合记录 cache coverage/window 或冷启动重同步 |
| 每端各写一套 ProjectionStore/BLoC/QuickJS 业务 reducer | temporaryId 对账、窗口排序、未读和权限会三端漂移；统一进 `helix-im` 的 InteractionProtocol |
| 把语义 `render_kind` 直接绑定具体 Component/Widget 类名 | 业务模块会反向依赖平台实现；`render_kind` 只能是稳定、平台无关的语义 renderer key |

---

## §5 不偏离机制

- **本文** = L2 常量，每次会话开局必读（设计理念锚点）。
- **AGENTS.md 顶部北极星指针** → 每次必加载，指回本文。
- **7 张 HX-C harness 卡** = 把 §1/§2 编译成 grep/CI/正反对照的**可执行护栏**（docs/harness/）。
- **decisions.md** = 已拍板的 ADR（防反复推导）。
- 改动与本文冲突 → 停下问；新决策 → 追加本文 §4 或 decisions.md。

---

## §6 抽象高度边界（语义交互可走 / 具体 UI 与 I/O 是红线）

> 2026-07-15 用户拍板：旧“方向 A 只能吐扁平数据 ViewModel”被 ADR-015 覆盖。
> 新边界的真源是 [`2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md`](./specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md)：统一的是跨端**语义交互合同**，不是具体 UI 工具包。

| 方向 | 含义 | 结论 |
|---|---|---|
| **A（语义交互 · 目标）** | `helix-im` 产出 `InteractionFrame(ViewMutation + ActionBinding + UiDirective)`：语义 view tree、稳定顺序、opaque action、focus/reveal/reset/dismiss/fallback 等跨端语义 | ✅ 三端统一实现；平台机械 apply、执行和 ack |
| **B（具体 UI · 红线）** | Rust 输出 DOM tag、Angular Component、Flutter Widget、CSS selector、像素、动画或平台对象生命周期 | ❌ 反向依赖平台，破坏 runtime-agnostic 与可替换 renderer |
| **C（往下 · 红线）** | core 自己做 I/O（省 driver） | ❌ 违反 §1.2 / §2（sans-IO）；三栖立刻碎，`HX-C001` 守门 |

**方向 A 不变量**：
- `SurfaceModel / ViewNode / ActionBinding / UiDirective` 是 IM 业务交互类型，只能在 `helix-im`；`helix-core` 只搬运不透明批次并提供确定性执行机制。
- `ViewNode.render_kind` 是语义 renderer key（如 `message.row.v1`），不得出现具体组件、Widget、DOM 或样式名；各端通过静态 registry 映射自己的组件。
- `ActionBinding.action_ref` 只暴露 opaque、surface-scoped 动作地址；权限、command 选择和 payload 组装留在 `helix-im`。
- `UiDirective` 只表达跨端语义“做什么”，平台决定“怎么做”；持久业务结果必须同时进入 ViewModel，不能只靠一次性 directive 成为权威。
- frame 必须 revision/epoch 可校验、确定性可回放、跨 FFI 批量传输；同一领域事实引起的多节点变化原子 apply。→ `HX-C007`
- shell 可以维护像素滚动、动画、原生焦点句柄和可丢弃 frame cache，但不得恢复业务 reducer、temporaryId 对账、read candidate、窗口排序或权限分支。

**时序硬约束**：InteractionProtocol 在基础 DomainModel、Persist barrier 与收发/同步链路之上增量实现；不得用纯文档协议掩盖当前 `declared/blocked` Gate，也不得让 UI 暂时代偿尚未进入 Helix 的业务规则。
