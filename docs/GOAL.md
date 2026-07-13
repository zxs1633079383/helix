# Helix — 项目北极星 / L2 目标常量（设计理念不变量，不可偏离）

> 这是 Helix 的**最高层目标与设计理念锚点**（harness 母本四件套之 GOAL.md，L2 常量）。
> 任何改动若与本文 §1 设计理念不变量 / §2 硬约束冲突 → **先停下问，不偏离**。
> 每次会话开局必读（加载顺序：本项目 AGENTS.md → **docs/GOAL.md** → SESSION → harness/README）。

---

## §0 一段话定义（北极星，背下来）

**Helix 是我们的「运行时无关确定性业务内核」**——目标是一份业务逻辑只写一次，产品目标集合为 **PC 端 Tauri（native）+ 手机端 Flutter（FFI）+ Web**；当前交付事实是 native/FFI 已产品化、Web 自 2026-07-13 按 ADR-014 进入 W0 可达性与契约验证，尚未交付。IM 只是跑在它上面的第一个业务模块，内核本身不认识任何业务；核心设计理念是 **sans-IO 六边形**：`helix-core` 零运行时、零平台依赖，它不做 I/O、不持有 tokio，只定义一组**端口（ports）**和一个纯状态机执行壳——喂入（入站帧 / now_ms / 端口响应）、吐出（Effect），真正的网络、存储、线程、时间、随机、ID 全部由各平台的薄驱动注入（这就是"确定性"的来源，也是并发不进 core 的原因），依赖方向严格单向 `driver → core` 永不反向；同一份 `helix-im` 负责三端共同的 WS 帧解析、sequence/cursor、echo、已读和补偿同步，平台只兑现 I/O。PC native 与 Flutter FFI 继续共同复用 `helix-driver-host` 的完整 SQLite/HTTP/WS；Web 规划由 `helix-driver-web` 在 Dedicated Worker 中兑现浏览器 Transport 和 session store，并只把近期消息、会话摘要、草稿或 UI 偏好投影为有界可淘汰缓存，不要求复制 PC/手机端的大体量数据库。现有 `helix-driver-wasm` 仍是历史实验，不等于正式 Web 产品壳。

---

## §1 设计理念不变量（违反任一 = 偏离北极星，必须停下）

1. **运行时无关**：`helix-core` 必须能被 PC native、Flutter FFI 与 Web WASM 共用，因此保持零 tokio/sqlx/reqwest/web-sys/tauri；业务模块同样只依赖 core ports，不直接依赖平台 runtime。→ `HX-C001`
2. **sans-IO / 六边形**：core 不做 I/O、不持运行时，纯状态机「喂入 → 吐 Effect」，I/O 由驱动做。
3. **确定性**：时间 / 随机 / ID 由 host 注入，core 不调 ambient `Instant::now`/`rand`/生成 id；同输入同输出可回放。
4. **并发不进 core**：core 单线程可跑；多线程是 native/FFI 驱动的事。Send 按 cfg 切，默认 Unsync。→ `HX-C002`
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
- ❌ 不逐条过 FFI 边界传 Effect（批量）→ `HX-C007`
- ❌ 不设互斥 feature → `HX-C004`
- ❌ 不让业务 schema 泄漏进 core 的 ports
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

---

## §5 不偏离机制

- **本文** = L2 常量，每次会话开局必读（设计理念锚点）。
- **AGENTS.md 顶部北极星指针** → 每次必加载，指回本文。
- **7 张 HX-C harness 卡** = 把 §1/§2 编译成 grep/CI/正反对照的**可执行护栏**（docs/harness/）。
- **decisions.md** = 已拍板的 ADR（防反复推导）。
- 改动与本文冲突 → 停下问；新决策 → 追加本文 §4 或 decisions.md。

---

## §6 抽象高度边界（方向 A 可走 / 方向 B·C 红线）

> 沿用 §1.6「业务不污染内核」+ §1.2「sans-IO」，把「core 还能往上吃多少 UI」这条线钉死。
> 完整设计 + cses-client 对照 + Crux 参照见 [`过会方案/helix-core-view-方向A设计.md`](./过会方案/helix-core-view-方向A设计.md)。

| 方向 | 含义 | 结论 |
|---|---|---|
| **A（往上一格 · 可走）** | core 吐**数据形态的 ViewModel**（数字 / 有序列表 / 高亮区间），各端 UI 绑定即渲染 | ✅ **未来增量切**。停在此线 |
| **B（再往上 · 红线）** | core 吐 **view 树 / 控件 / 像素**（Rust 写 UI） | ❌ 退化成 Dioxus；现网 Angular 全废；见 §4 已类比否定 |
| **C（往下 · 红线）** | core 自己做 I/O（省 driver） | ❌ 违反 §1.2 / §2（sans-IO）；三栖立刻碎，`HX-C001` 守门 |

**方向 A 不变量（不破 §1.6 的前提下才放行）**：
- `ViewModel` 是**业务类型 → 只在 helix-im**（同 DomainEvent / cursor 待遇）；`grep ViewModel crates/helix-core` 必须 = 0。
- core 只新增**通用**机制：`Effect::Render`（fire-and-forget 通知）+ `Module::view() -> ViewBytes`（不透明字节，core 不解释内容，对照 Crux `Core::view` 泛型）。
- core 可以说「未读=3」「这段是 mention」，**不能说**「画一个红色 badge / 渲染第几个像素」——越线即方向 B。
- ViewModel 跨边界**必须 diff/patch 或窗口化**，不全量 ship → 复用 `HX-C007`。

**时序硬约束**：方向 A 必须**在基础 core + Model + 收发/同步链路稳固之后增量切**（先 D 后 A）；ViewModel 投影的是业务状态，链路没进 core 之前没有干净 Model 可投影。
