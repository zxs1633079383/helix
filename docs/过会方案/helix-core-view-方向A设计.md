# Helix core-view —— 方向 A（ViewModel 投影接管）设计

> **现行性说明（2026-07-15）**：本文是“数据 ViewModel 接管”的历史阶段方案，不再定义 Helix Headless 的最终高度边界。ADR-015 与 [`GOAL.md` §6](../GOAL.md) 已把目标升级为完整 `InteractionProtocol`：ViewModel 只是 `InteractionFrame` 的可重放呈现状态，还必须统一 `UiEvent/HostObservation`、`ActionBinding`、`UiDirective`、ack/resync。本文 §3 中“view tree/控件一律属于方向 B 红线”的旧结论已被覆盖；现行红线是**禁止具体 DOM/Component/Widget/CSS/像素/动画实现进入合同**。实现与评审请以 [`UC01-16 跨端交互协议 SPEC`](../specs/2026-07-15-uc01-16-cross-platform-interaction-protocol-spec.md) 为准。

> 抽象高度上「往上一格」：core 不止吐领域事件，还吐**给 UI 的只读投影 ViewModel**，各端 UI 退化成 dumb 渲染。
> **可行性结论：机制层可行（Crux 是活的存在性证明），但「可行 ≠ 无风险」。** 风险不在「能不能做」——`view()` 是 `Model` 纯函数、`Render`+typegen 都有现成范式；风险几乎全压在 **§4 红线1：大列表 ViewModel 的增量投影/传输（性能工程）**——这恰是 Crux 原生范式（每次 Render 整份重拉）覆盖最弱处，必须自建窗口化 / diff-patch（方案见 §4.1）。另一条是 **§6「先 D 后 A」前置时序**：没有干净的 in-memory Model 之前无对象可投。守住 §3 一条线则不撞北极星。参照实现 = Crux（`/Users/mac28/workspace/rustWorkspace/crux`）。
> 上层约束见 [`../GOAL.md` §6 抽象高度边界](../GOAL.md)；机制流程图见 [`helix-core-im-架构设计.md` §七](./helix-core-im-架构设计.md)；边界纪律见 [`../harness/HX-C007`](../harness/HX-C007-effect-batch-across-boundary.md)。
> **时序定位：方向 A 在「基础 core + Model + 收发/同步链路」稳固之后增量切，不先行。**

---

## §0 一句话

把现在散在 Angular（排序 / 未读聚合 / 分组 / 草稿 / 昵称合并）的**展示派生逻辑**，搬进 helix-im 的纯函数 `view(&Model) -> ViewModel`，一次写好三端一致、可回放测试；各端只订阅一个 ViewModel 直接绑模板。

---

## §1 方向 A = 把 helix 补成 Crux 的完整形态

Crux 的 `App` trait 就是 helix 走到方向 A 终点的样子（`crux_core/src/lib.rs`）：

| Crux `App` | helix 现在 | 方向 A 要补 |
|---|---|---|
| `type Model`（完整内部状态） | Module 有状态，无统一 Model | helix-im 持有 IM **Model**（频道/消息/未读/成员/草稿） |
| `update(event,&mut model)->Command<Effect>` | ✅ Module + Effect(Persist/Send/ScheduleTimer/Emit) | 已具备 |
| **`view(&model)->ViewModel`** | ❌ 无 | ★ helix-im 加纯函数 `view()` 投影展示态 |
| `type ViewModel`（UI 只读投影） | ❌ 无 | ★ ViewModel 定义在 **helix-im**（业务，不进 core） |
| Render capability → shell 拉 `view()` | EventSink 吐**领域事件** | core 加通用 `Render` effect；host 收到拉 `view()` |
| typegen 生成 Swift/Kotlin/**TS** | ❌ 无 | ★ 对 cses-client 高价值：生成 ViewModel 的 TS 类型 |

> **helix 今天 = Crux 去掉 `view()`/`ViewModel`。方向 A = 照 Crux 补回来。** core 永远不认识 ViewModel 内容（同 DomainEvent 待遇，GOAL §1.6）。

### §1.1 Crux 三端边界 ↔ helix 映射

Crux 的三端边界**只有一条、三端共用**：一套字节协议 + 一套 typegen 类型 + 各端薄 shell 执行 Effect。core 是同一份，三端只换「Effect 的执行实现」和「ViewModel 的渲染框架」。

| Crux 边界构件 | 作用 | helix 对应 |
|---|---|---|
| `Bridge.update/resolve/view`（`crux_core/src/bridge/mod.rs`，bincode `[u8]` 进出） | UI 事件进 core / Effect 结果喂回 / 拉 ViewModel | `step(Tick)->Effect` + 未来 `Module::view()` |
| Capability `Request<Op>`（值，不执行） | core 只**描述** Effect | `Effect` 单一 enum（Persist/Send/ScheduleTimer/Emit/**Render**） |
| `Effect::Render`（fire-and-forget）→ shell 拉 `view()` | 通知「投影变了来拉」 | `Effect::Render` → host 拉 `Module::view()` |
| `typegen` 生成 Swift/Kotlin/**TS** 类型 | 编译期跨语言类型契约 | 抄此套生成 ViewModel 的 TS interface |
| bincode `[u8]` 过边界 | 语言无关传输 | **HX-C007「Effect 跨边界批量传不逐条」** |
| 各端 shell 执行 Effect（差异在此，不在边界上） | URLSession/OkHttp/fetch · SwiftUI/Compose/React | 三套 driver（native/wasm/ffi）实现 ports |

> **唯一结构差异**：helix 的 `Effect` 是**单一 enum**（已拍板，过会纪要③），Crux 是 capability 派生的 Effect enum；其余同构。
> 三端"差异"只在**边界之下**（Effect 执行实现 + 渲染框架），边界协议（Bridge 三方法 + typegen 类型）三端字节级一致——这就是「一份 core 骑三端」的来源。

---

## §2 结合 cses-client 现状：接管前 → 接管后

**现状有利**：业务链路（gate 状态机 / cursor / 去重 / 持久化 / 离线 sync / bitmap）**已在 Rust**（`src-tauri/features/im`）；剩下要接管的是 TS 里的**纯派生展示逻辑**。

**今天（方向 A 之前）——展示逻辑在 Angular 算：**
```
Rust(features/im) ──18 个细粒度事件──▶ im:__bus__ ──▶ ipcEvent.service(RxJS Subject)
   im:post:received / im:channel:update / im:post:read ...        │
                                                                   ▼
                                          ipcEventHandler.service(业务处理)
                                                                   ▼
   Angular 自己算: ① dialogList 排序(pinnedTop + updateAt)
                   ② 未读角标聚合(calculateVisibleUnreadTotal,排除 type='T')
                   ③ 消息分组 / @mention 高亮 / 草稿 draftMap / 成员昵称合并
                   ④ messageGlobalBaseContext 一堆 Signal/BehaviorSubject
                                                                   ▼
                                          组件渲染
```

**方向 A 接管后——helix-im 持有 Model + view()，Angular 退化成 dumb 渲染：**
```
helix-im(Model: 频道+消息+未读+成员+草稿) ──update()──▶ 状态变
                                                            ▼ view() 投影(纯函数)
   ViewModel {
     orderedDialogs[]                  // 已排好序(pinned + lastMsgTime),Angular 不再排
     totalUnread, perChannelUnread{}   // 已聚合好,Angular 不再算
     currentConversation: { groupedMessages[], mentionRanges[] }  // 已分组/已标好高亮区间
     draftText                         // 草稿(还能三端同步)
   }
                                                            ▼ Render effect
   Tauri host 收到 Render ──▶ core.view() ──▶ im:__bus__ 发 im:viewmodel:patch
                                                            ▼
   Angular: 订阅 1 个 ViewModel,直接绑模板。没有排序/聚合/分组/草稿逻辑了
```

**归属切分（这条线就是方向 A 安全边界，越过 = 方向 B 退化）：**

| 现在在 TS | 方向 A 后归属 | 说明 |
|---|---|---|
| dialogList 按 pinnedTop + updateAt 排序 | → helix `view()` | 语义排序，三端应一致 |
| 未读角标聚合 `calculateVisibleUnreadTotal` | → helix `view()` | 纯派生，排除 topic 规则进 core |
| 消息分组（按发送人/时间聚合） | → helix `view()` | 语义分组 |
| @mention 高亮**区间** | → helix `view()` 出 ranges | ranges→`<span>` DOM 渲染留 Angular |
| 草稿文本存储 `draftMap` | → helix **Model**（可三端同步） | 实时键入留 Angular，防抖后提交进 core |
| 成员昵称合并 / 可见性过滤 | → helix `view()` | 语义投影 |
| 频道列表**虚拟滚动窗口化** | 留 Angular | helix 给有序列表/窗口切片，windowing 是 UI 的事 |
| tab 高亮 / 滚动位置 / 动画 / CSS | 留 Angular | 纯视图态，不进 core |
| 文件上传进度 UI / 通知弹窗渲染 | 留 Angular | 平台 UI |

**消掉的东西**：`ipcEventHandler` 大部分、18 个细粒度 `im:*` 事件、`messageGlobalBaseContext` 的 Signal 群、TS 排序/聚合/分组函数——大半溶进 helix-im。

---

## §3 安全边界：方向 A 可走 / 方向 B·C 红线

| 方向 | 含义 | 判定 |
|---|---|---|
| **A（往上一格）** | core 吐**数据形态的 ViewModel**（数字/有序列表/区间），UI 绑定 | ✅ 可走，停在此线 |
| **B（再往上）** | core 吐 **view 树 / 控件 / 像素** | ❌ 退化成 Dioxus；现网 Angular 全废；撞 GOAL §4 已否定 |
| **C（往下）** | core 自己做 I/O（省 driver） | ❌ 违反 GOAL §1.2 / §2（sans-IO）；三栖立刻碎，HX-C001 守门 |

**一句话判据**：core 可以说「未读=3」「这段是 mention」，**不能说**「画一个红色 badge 元素 / 渲染第几个像素」。

---

## §4 三条工程红线（cses-client 专属，不守住会翻车）

1. **绝不每次变更 ship 整个 ViewModel**。dialogList 几百、消息上千，每条新消息全量序列化过 IPC = 灾难。→ **ViewModel diff/patch** 或 **窗口化 ViewModel**（只投影可见会话 + dialog 切片）。这把 HX-C007「Effect 跨边界批处理」从建议升级成生死线。
2. **虚拟滚动留 Angular**。helix 出数据（有序列表/窗口切片），Angular 出窗口（windowing + 渲染）。helix 绝不碰像素 = 不退方向 B。
3. **用 typegen 生成 ViewModel 的 TS 类型**（抄 `crux_core::typegen`）。Angular 拿到编译期类型安全的 ViewModel interface；改 Rust 字段、TS 立刻红——三端契约的巨大收益。

### §4.1 大列表投影性能方案（§4 红线1 展开 · 分层叠加而非五选一）

**先决条件（不具备就别上 B/C/D/E，会翻车）：**
1. 每行稳定 id（dialog id / message `temporaryId`+`id`）——diff / 窗口定位全靠它（cses 已有 ✅）
2. Model 能增量感知「什么变了」（revision 计数 / per-section dirty flags / `update` 返回 changed-set）——**需建**
3. ViewModel 形状 diff 友好：keyed 扁平集合，别每次重建整棵树
4. viewport（offset/limit/当前 channelId）能作为 `Tick::Command` 进 core——窗口化前提
5. patch / 窗口协议 typegen 化（三端统一 apply，不各写一遍）——抄 `crux typegen`
6. 守 HX-C007 批次边界 + 丢补丁兜底（patch 序号断 → 全量 resync，类比 `too_long`）

| 方案 | 做法 | 算量/传输 | 优 | 缺 | 适用 |
|---|---|---|---|---|---|
| **A** 全量重投影+全量 ship（Crux 原生） | `view()` 算整份 → 整份序列化过边界 | O(N)/每次变更 | 最简、零 diff、确定性最干净 | 大列表必崩 | 小列表 / 首屏 / 兜底基线 |
| **B** 窗口化投影 | 只投影可见切片，viewport 进 core | O(可见行)≈常数 | 与总量无关、天然配虚拟滚动 | 全局聚合（总未读）不能窗口化；跳转/搜索定位要额外处理 | **chat 主力** |
| **C** diff/patch | core 存上次快照，只 ship added/removed/updated/moved（by key） | O(变化量) | 「列表大但每次只变几条」最优（IM 正是） | core 2x 内存或重算；apply 逻辑；丢补丁要兜底 | 与 B 组合 |
| **D** 分块多通道 VM | 拆 unread / dialogList / conversation / draft 各自独立 `Render` | 各块用各自最优 | 改一块不动其他、增量落地友好 | 多通道要解复用（`im:__bus__` 信封天然支持） | **顶层组织方式** |
| **E** 脏标记惰性投影 | revision + dirty flag，只对脏块调 `view()` | 省 CPU（不重算干净块） | 没变不重算 | 漏标=UI 不更新 / 错标=多算，正确性要严守 | B/C/D 的 CPU 优化层 |

**推荐组合（分层叠，不是五选一）：**
- 顶层 **D 分块**：`unread` 标量走 **A**（极小全量）；`dialogList` 走 **B+C**（窗口化 + 窗口内 diff）；`conversation` 走 **B**（消息窗口化）；`draft` 走 **A**（单字段）。
- 全局加 **E**（脏标记省重算）+ **A 兜底**（patch 断链/版本不齐 → 退该块全量）。
- 落地顺序对齐 §6 一刀一个：第1刀未读用纯 **A** 验证 Render/typegen/patch 管线 → 第2刀 dialogList 上 **B** → 列表稳了再叠 **C** → conversation 上 **B**。**先证明管线，再上性能。**

> 红线不变（§4 红线2）：以上全是「core 出数据/切片/补丁，UI 出窗口/像素」。windowing / DOM / 滚动位置永远在 Angular，任何方案都不许 core 碰像素 = 不退方向 B。

---

## §5 落地接口草案（ViewModel 在 module，core 只给通用机制）

**core 侧（通用，不认识业务）**——给 `Module` 加可选投影 + 新增 `Render` effect：
```rust
// helix-core：通用机制，core 不知道 ViewModel 形状（同 Crux Core::view 泛型）
pub trait Module {
    // ...既有 step/handle...
    /// 可选：把当前模块状态投影成不透明可序列化字节（host 拉取用）
    fn view(&self) -> Option<ViewBytes> { None }   // ViewBytes = 已序列化的投影，core 不解释内容
}

pub enum Effect {
    Persist(/* ... */), Send(/* ... */), ScheduleTimer(/* ... */), Emit(/* DomainEvent */),
    Render,            // ★ 新增：通知 host「投影变了，来拉 view()」（fire-and-forget，对应 Crux RenderOperation）
}
```

**helix-im 侧（业务，ViewModel 住这）**：
```rust
// helix-im：Model + ViewModel + 纯函数 view()
struct ImModel { /* 频道 / 消息 / 未读 / 成员 / 草稿（从 Storage port 投影的 in-memory 状态）*/ }

#[derive(Serialize)]            // typegen → TS interface
struct ImViewModel {
    ordered_dialogs: Vec<DialogVm>,
    total_unread: u32,
    per_channel_unread: HashMap<ChannelId, u32>,
    current_conversation: Option<ConversationVm>,  // grouped_messages + mention_ranges
    draft_text: Option<String>,
}

impl ImModule {
    fn view(&self) -> ImViewModel { /* 纯函数：排序/聚合/分组/区间，全在这里，确定性可回放 */ }
    // update 内状态变更后，吐 Effect::Render 通知 host
}
```

**host 侧（Tauri/wasm/ffi 薄壳）**：收到 `Effect::Render` → 调 `module.view()` → 经现有 `im:__bus__` 发 `im:viewmodel:patch` → Angular 订阅绑定。

> 与 GOAL §1.6 一致：`ImViewModel` 是业务类型，**只在 helix-im**；core 的 `Render`/`ViewBytes` 是通用原语，`grep ViewModel crates/helix-core` 应为 0（同 cursor/DomainEvent 隔离判据）。

---

## §6 时序：先基础 core，再 core-view，最后增量切 ViewModel

```
方向 D 阶段1(已做一半): 离线同步 / cursor / gate         ✅ 在 Rust
方向 D 阶段2(接下来):    收发消息链路全进 helix-core(从现网 im handlers 搬,行为真源)
基础 core 完工:          ports + step 循环 + Module + 【in-memory Model】 建好
core-view 机制:          core 加 Effect::Render + Module::view()；host 加 view() 拉取链路
方向 A 增量切(一刀一个小 ViewModel,不一次性):
   第1刀  未读角标         ← 最小 VM,验证 Render/typegen/patch 管线
   第2刀  dialogList 排序
   第3刀  消息分组 + mention ranges
   第4刀  草稿            ← 顺带拿到三端草稿同步
```
**为什么必须先 D 后 A**：ViewModel 投影的是业务状态；收发/同步没进 core 之前没有干净 Model 可投影。**为什么必须增量**：每刀替掉一个 TS 计算、独立验证、可回滚。

---

## §7 回报

- **写一次三端一致**：未读怎么算 / 列表怎么排 / 消息怎么分组——今天 Angular 一份，未来 iOS Swift、Android Kotlin 各一份必漂移；`view()` 写进 helix-im 一次，三端 ViewModel 字节级一致。
- **可回放测试**：`view` 是 `Model` 的纯函数，给定事件日志 → 精确 ViewModel，Angular 的 Signal 汤做不到。

---

> 交叉引用：北极星边界 [`../GOAL.md` §6](../GOAL.md) · 机制流程图 [`helix-core-im-架构设计.md` §七](./helix-core-im-架构设计.md) · 边界批处理护栏 [`../harness/HX-C007`](../harness/HX-C007-effect-batch-across-boundary.md) · 参照实现 Crux `crux_core/src/lib.rs`（`App::view`/`Core::view`/`typegen`）。
