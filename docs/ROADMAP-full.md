# Helix 完整 Roadmap（D + A 全景 · crux-grounded · headless 守卫逐阶段）

> 2026-06-15 · 本文是**全景 + 设计推演**视角；天级 todo / 验收 checklist 见 [`ROADMAP.md`](./ROADMAP.md)（两者分工：本文管"为什么这么排、每阶段守什么线"，ROADMAP.md 管"这周做哪几条"）。
> 2026-06-15 更新（ADR-006）：当前产品路线收缩为 PC Tauri + Flutter 手机端；Web/WASM 归档暂停。本文中旧 wasm 阶段仅作历史推演，不作为当前 roadmap。
> 关联：北极星 [`GOAL.md`](./GOAL.md) · 决策权威 [`harness/decisions.md`](./harness/decisions.md) · 机制权威 [`过会方案/helix-core-im-架构设计.md`](./过会方案/helix-core-im-架构设计.md) · 方向A [`过会方案/helix-core-view-方向A设计.md`](./过会方案/helix-core-view-方向A设计.md) · 参照实现 Crux（`/Users/mac28/workspace/rustWorkspace/crux`，源码已核） · 行为真源 cses-client（`src-tauri/features/im` + `message-v3`）。

---

## §0 贯穿不变量（铁律：core 与 core-view 永远是 headless 通用核）

> 这是全篇的轴心。**每个阶段出门前都要过这道闸门**——它证明 core 没被 IM 污染、没碰 I/O、没碰像素，始终是「运行时无关的通用核」，IM 只是 guest #1。

| 闸门 | 判据（machine-checkable） |
|---|---|
| 零运行时依赖 | `cargo tree -p helix-core \| grep -iE 'tokio\|sqlx\|reqwest\|web-sys\|tauri'` = 空 |
| 零业务概念 | `grep channel/message/temporaryId crates/helix-core` = 0；公开面 `cargo public-api` 对 allowlist 无新业务类型 |
| 双端编译绿 | native 产品 crate（core/im/driver-native/driver-ffi）全过；wasm 不作为当前必跑目标 |
| 加模块/加端 core 零改 | 加模块 = `Module`+`register`；加端 = 实现 ports（开闭原则） |
| 确定性可回放 | 同 Tick 序列 → 同输出 |
| **（进 A 后追加）** | `grep ViewModel crates/helix-core` = 0 —— ViewModel 是业务类型，只在 helix-im |

**Crux 源码佐证 headless 是真的可达**：`Core<A>` 仅持 `model + app + 一个 yield-effects 的执行器`，零 I/O、零 async runtime、零渲染；Capability 只是返回 `Command` 的构造器，不执行。helix 守同一条线，且更严（见 §5）。

---

## §1 两轴正交模型（core 永在交叉点不动）

```
横向（加业务模块，core 零改）:  IM ──→ +Presence ──→ +Meeting ...
纵向（加平台端，core/模块零改）: native ──→ ffi（Flutter）
        交叉点 = helix-core（永远 headless 通用核） → M×P 塌成 M+P

core-view（方向 A）= core 多吐一种通用 Effect(Render) + 一个泛型投影口(view)，
   不改变它 headless / 通用 / 运行时无关的本质——它仍在交叉点上不动。
```

---

## §2 阶段总览

| Phase | 名称 | 轴 | 状态 |
|---|---|---|---|
| 0 | 骨架（5 crate / 7 trait / step 壳 / Module host） | core | ✅ 完成 |
| 1 | 先证最硬骨头（乱序 backfill proptest） | 业务① | ✅ 完成 |
| 2 | native 落地 + 真 Go 联调 + Tauri 换引擎 | 业务① + native | 🔄 M2 ~90%（仅 Tauri 四接缝剩余；2a/2b/2c ✅） |
| 3 | 基础 core 完工 + 统一 in-memory Model | 业务①（D 阶段2 收尾） | ⏳ A 的枢纽前提 |
| 4 | FFI / Flutter 手机端 | +端 | 🔄 M3-F4 启动（F2/F3 ✅，F4 engine 接线拆解中） |
| 5 | wasm 浏览器端 | +端 | archived（ADR-006 暂停维护） |
| — | **以上 = 方向 D 当前产品完成：业务①写一次 PC + Flutter 双端一致，地基独立回本** | | |
| 6 | core-view **通用机制**（Render + view + typegen） | core（仍 headless） | 🔮 未来 |
| 7 | 方向 A 增量切（一刀一 ViewModel）+ 大列表性能自研层 | 展示②（helix-im） | 🔮 未来 |
| 支线① | AI 工程化硬化（CI matrix / clippy deny / public-api gate） | 工程化 | 并行 |
| 支线② | 第二模块验证（presence / meeting，core 零改） | 通用性裁决 | D 后任一时点 |

---

## §3 逐阶段详述

### ✅ Phase 0 — 骨架
**产物**：workspace 5 crate · 5 类 Port / 7 trait · `step(Tick,now_ms)→&[Effect]` 纯函数壳 · Module host（accepts/handle/on_start/on_stop + register/dispatch）· Effect 7 variant / Tick 6 variant。
**headless 守卫**：`cargo tree` 空已实测；core 零业务。

### ✅ Phase 1 — 先证最硬骨头（纪要头号风险）
**产物**：乱序 backfill 的 proptest——gate / cursor 单调 / 不丢不重 / 确定性。**先证再写 driver**：在最脏的「乱序帧缓冲 + gap 检测 + 按序补齐」链上把架构钉死，过会底气来自此。

### 🔄 Phase 2 — native 落地 + 真 Go 联调 + Tauri 换引擎（M2）
| 子阶段 | 状态 | 内容 | grounding |
|---|---|---|---|
| 2a 业务①搬进 helix-im | ✅ | `handlers/*` + `state.rs GateState` → `handle` + owned 状态；后续 WS registry 重构（main 线）+ sync-v2 全链路 | cses `src-tauri/features/im` 逐文件 |
| 2b driver-native 实现 ports | ✅ | Transport/Storage/Http 落地；`BoundedSpawner` 背压根治（Persist/Http/HttpFire 统一） | cses 现网并发形态 |
| 2c UC-1 前置4块 + 真 Go smoke | ✅ | hello 握手 / B4 event_seq / B5 phantom / echo→reconcile + golden fixtures；真消息端到端验过（2026-06-17） | panel-review 揪出的死/缺/脆 |
| 2d Tauri 四接缝 + 灰度 | ⬜ **未做（M2 唯一剩余）** | 10 命令内部切 Tick / 读旁路 · `NativeEventSink`→`im:__bus__` · 同库灰度 · `store:*` 对齐（B3 readBits ✅） | **Angular message-v3 零改** |

**headless 守卫**：业务①全在 helix-im，平台胶水全在 driver-native，core 零改。交付物 = **换引擎，不是重写客户端**。

### ⏳ Phase 3 — 基础 core 完工 + 统一 in-memory Model（★ A 的枢纽）
**产物**：收发链路剩余 handler 全搬进 core；helix-im 建**统一 in-memory Model**（频道/消息/未读/成员/草稿，从 Storage port 投影的内存态）。
**为什么单列**：**这是「先 D 后 A」的转轴**——ViewModel 投影的是业务 Model，没有干净 Model 之前 A 无对象可投。Model 没建好坚决不碰 Phase 6/7。
**headless 守卫**：Model 是 helix-im 业务状态，core 只见 `table:&'static str`，不知是「消息」还是「草稿」。

### 🔄 Phase 4 — FFI / Flutter 手机端（M3，F4 启动）
C ABI + Dart FFI 最小 API → rusqlite/SQLite 存储 → `helix_command`/`helix_query` + Dart Stream 事件流 → `panic=abort` cdylib/staticlib（HX-C006）→ Flutter 冒烟 → native vs FFI 同 Tick 剧本一致。
**进度**：F2（FfiStorage 复用 host）✅ · F3（shared HTTP/WS client + set/remove_header + FfiTransport）✅ · opaque handle/create/destroy ✅；**F4（engine 接线 + 事件回 Dart Stream）2026-06-18 启动**——核心缺口 = FFI 无 engine 泵（`helix_on_ws_frame`/`on_http_response` 不喂 engine、无 command 入口）。**逐子任务拆解（F4.1 泵归属决策 / F4.2 runtime+Tick 入队 / F4.3 事件回 Stream / F4.4 panic+spawn_blocking 核 / F4.5 Flutter 冒烟）见 [`ROADMAP.md` §一 M3 ▶ F4 实装拆解](./ROADMAP.md)**。
**headless 守卫**：同一份 core 跑 ffi 零改；panic 不跨 `extern "C"`；opaque ptr 单 owner。

### archived Phase 5 — wasm 浏览器端（暂停维护）
IndexedDB / Worker / OPFS / wasm 包体积相关方案按 ADR-006 归档。当前不做网页端、不补 driver-wasm、不纳入 CI 或交付验收；未来重启 Web 需新增 ADR 重新评估 cache/storage 策略。

> **—— 到此 = 方向 D 当前产品完成。业务①（收发/同步/对账/离线）写一次 PC Tauri + Flutter 双端一致、可回放可单测。这是地基，独立回本——即使 A 不做，投资也不白费（纪要定锚）。——**

### 🔮 Phase 6 — core-view **通用机制**（仍然 headless）
core **只**加两个**通用**原语：
- `Effect::Render`（fire-and-forget 通知，对照 Crux `RenderOperation`，Output=()）；
- `Module::view() -> Option<ViewBytes>`（**不透明字节，core 不解释内容**，对照 Crux `Core::view` 泛型）。

host 加 `Render → view() → im:viewmodel:patch` 链路；typegen 抄 `crux TypeGen::register_app` 先出 ViewModel 的 TS interface。
**headless 守卫（升级）**：`grep ViewModel crates/helix-core`=0；public-api 只新增 `Render`/`ViewBytes` 两个**通用**条目，无任何 IM ViewModel 类型。**core 加了「投影机制」，但不知道投影内容——它还是通用核**；第二模块（presence）同样能用 `Render`+`view()` 投自己的 ViewModel，core 一行不改。

### 🔮 Phase 7 — 方向 A 增量切 + 大列表性能（**自研层，非抄 Crux**）
| 刀 | 内容 | 性能方案（[方向A §4.1](./过会方案/helix-core-view-方向A设计.md)） |
|---|---|---|
| 第1刀 | 未读角标 | 纯 **A** 全量（验证 Render/typegen/patch 管线，此时与 Crux 等价） |
| 第2刀 | dialogList 排序 | **B** 窗口化（**自研起点**） |
| 第3刀 | 消息分组 + mention ranges | **B**+**C**（窗口内 diff） |
| 第4刀 | 草稿 | 纯 **A** + 进 Model（顺带三端草稿同步） |

顶层 **D 分块** + 全局 **E 脏标记** + **A 兜底**。
**关键**：Crux 源码确认 `view()` 每次**整份** ship ViewModel、零 diff/windowing（见 §6）——所以大列表增量投影是 helix **在 Crux 之上自研的一层**，roadmap 单列预算，**不算进「抄 Crux」**。
**headless 守卫**：ViewModel 在 helix-im；windowing/DOM/像素永远在 UI——**core 出数字/切片/补丁，不碰像素 = 不退方向 B**。

### 支线① — AI 工程化硬化（可与任意 Phase 并行）
CI 双端 matrix → clippy deny-list（`Instant::now`/`println!` 编译期禁）→ per-crate CLAUDE.md → `cargo public-api` gate。**把 §0 headless 三证从「手跑/会话纪律」升级成「编译期/CI 闸门」**——这才是不变量真正被守住的方式（对照 Bun：约束嵌进工具链）。落地动作见 [`ROADMAP.md` §二 P2 工程化](./ROADMAP.md)（原「AI工程化对齐-Bun-checklist」已随 docs 缩减并入 ROADMAP）。

### 支线② — 第二模块验证 = 通用核裁决（D 后任一时点）
`register(MeetingModule)` / `register(PresenceModule)`，**core 零改**跑通。会议用全四能力但形状完全异于 IM（无 temporaryId 对账、无 cursor 同步）；audit 只用 Http+Storage+Timer 不碰 WS。**这是「core 是通用核而非 IM 框架」的代码裁决**（架构设计 §六）——你最在意的不变量，要用第二个形状异于 IM 的模块插得进来证明，不靠嘴说。

---

## §4 Crux 构件映射（Phase 6/7 对照真实源码）

> 源码已核（`crux_core` 0.19）。helix 今天 = Crux 去掉 `view()`/`ViewModel`/`Render`/typegen 四件。

| Crux 真实构件（源码路径） | helix 现状 | Phase 6 要补 |
|---|---|---|
| `App{Model,Event,ViewModel,Effect}` + `update→Command<Effect,Event>` + `view(&model)→ViewModel`（`crux_core/src/lib.rs`） | `Module` + `step(Tick)→&[Effect]`（无 view） | helix-im 加纯函数 `view()` |
| `RenderOperation`（Output=()）fire-and-forget（`capabilities/render.rs`） | EventSink 吐领域事件 | core 加 `Effect::Render` 通用变体 |
| `Bridge::{update, resolve, view}`，bincode `[u8]`（`bridge/mod.rs`） | `step` + corr 回灌（无 view 出口） | host 加 `view()` 拉取出口 |
| `EffectId`（u32 Slab 索引）做 effect 续算配对（`bridge/registry.rs`） | `corr_map` ✅ 同机制 | 复用，不改 |
| `TypeGen::register_app` → Event + ViewModel + EffectFfi + Request 的 Swift/Kotlin/TS/C#（`type_generation/serde.rs`） | 无 | 抄 typegen，先出 TS |
| shell 三责：forward event / execute effect+resolve / pull view（`examples/counter/.../ffi.rs`） | 三端薄桥（Tauri/wasm/ffi） | 不变 |

---

## §5 helix ⊄ Crux：有意的分歧（别盲抄）

| 维度 | Crux | helix（已拍板） | 取舍 |
|---|---|---|---|
| Effect 形态 | `#[effect]` 宏按 Operation 派生变体 | **单一手写 enum**（纪要③） | helix 更简；放弃 capability 模块化，可接受 |
| update 返回 | `Command<Effect,Event>`，**core 内含 async executor** | `step(Tick)→&[Effect]`，**core 内零 async executor** | **helix headless 更彻底**：执行器/并发全在 driver，core 纯 `match` 单线程可跑，更利三栖 |
| 输入 | `Event` 单类 | `Tick` 六类（Command/Inbound/PortReply/Timer/Connected/Disconnected） | helix 把网络入站/连接生命周期显式化，贴 IM |

> **抄 Crux 的是**：沙漏形状 + `Render`/`Bridge`/typegen 三件套 + EffectId↔corr 配对范式。
> **不抄的是**：它的 `Command` async executor——helix 故意不把执行器放进 core，这正是 helix headless 比 Crux 更纯之处。**这条要守住，别被「crux 有 Command 我们也加」带偏。**

---

## §6 大列表投影性能 = 自研层（Crux 不白送，单列预算）

Crux 源码实证：`Core::view()` 就一句 `self.app.view(&model)`，bincode **整份** ViewModel 过 Bridge，**crux_core 零 diff / 零 patch / 零 windowing**——官方做法是「整份 ship + shell 自己虚拟滚动」。对 chat 千条消息不够。

所以 Phase 7 的分层方案（[方向A §4.1](./过会方案/helix-core-view-方向A设计.md)：A 全量 / B 窗口化 / C diff-patch / D 分块 / E 脏标记）是 helix **在 Crux 之上自研**：第1刀（未读，纯 A）与 Crux 等价，**第2刀（dialogList）起才进入自研窗口化**。分界点清晰，预算单列。

---

> 交叉引用：北极星 [`GOAL.md`](./GOAL.md) · 决策 [`纪要`](./harness/decisions.md) · 机制 [`架构设计`](./过会方案/helix-core-im-架构设计.md) · 方向A [`方向A设计`](./过会方案/helix-core-view-方向A设计.md) · 天级 todo [`ROADMAP.md`](./ROADMAP.md) · 护栏 [`harness/`](./harness/README.md) · 文档地图 [`README.md`](./README.md)。
