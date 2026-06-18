# Helix EventBus 设计（PUB 单向 + REP 双向 + 下游能力契约）

> 合并自 `helix-eventbus-rep-双向通道设计.md`（PUB/REP 判据 + 方向①②场景）+ `helix-eventbus-下游能力契约.md`（消费者反推的信封契约），2026-06-17。
> 归属：MVP1（Rust-Core 与二端 Driver 骨架打底）的 EventBus 能力补全 + 能力定线。
> 机制权威源对齐 [架构设计](./helix-core-im-架构设计.md) §3.1；红线对齐 [GOAL](../GOAL.md) §1.6（业务不污染内核）/ §6（抽象高度边界）；已拍板结论对齐 [decisions.md 过会拍板节](../harness/decisions.md)（`Effect` 单一 enum、四能力行为级解耦、DomainEvent 版本化）。
> 代码落点：`crates/helix-core/src/{effect.rs,tick.rs,engine.rs,ports/event_sink.rs}`；helix-im 信封序列化层；回放测试 `crates/helix-core/tests/request_reply_replay.rs`。

---

## §0 EventBus 的两种语义 + 判据

EventBus 有两种语义，设计每个事件前先过判据决定走哪条：

- **PUB（单向广播）**：`Effect::Emit` → `EventSink::emit()`，fire-and-forget，发了就走，无回音。下游能力契约（§3–§6）的主体在这条线。
- **REP（请求-应答，双向）**：发起方发一个请求，**必须**拿到一个与之一一对应的回答才能推进状态机；靠 `Correlation`（corr）把「问」和「答」配起来（§7–§9）。

**PUB/REP 判据（设计每个事件前先过）**：
> 「发出后，发起方是否必须拿到一个**与这次请求一一对应**的回答，才能推进自己的状态机？」
> 是 → REP；否 → PUB。
> 且：如果那个回答本来就来自**存储 / HTTP 端口**，用现成的 `Persist{corr}` / `Http{corr}` 回路，**不要新造 `Request`**——REP（`Effect::Request`）只服务 "core ↔ 宿主/UI" 这条线。

---

# 第一部分 · PUB —— 下游能力契约（消费者反推）

## §1 设计驱动方向的反转

**普通做法（生产者驱动）**：我们 emit 什么 → 下游爱用不用。
**本契约（消费者反推）**：先问「下游某个域要拿这条事件去干什么、缺哪些上下文才**不必回头调我们**」→ 反推出事件**必须自带**什么能力与上下文 → 我们吐出去的每条事件对下游**自解释、有价值**。

> **反推判据（设计每条事件前先过）**：
> 「一个下游域，只拿到这条事件本身（不查 DB、不调接口、不回问 core），能不能完成它的工作（弹通知 / 加徽章 / 进 @我流 / 记审计）？」
> 能 → 上下文够；不能 → 缺的字段就是本契约要补的。

**为什么这能「一次操作引发无限内容」**：一条**上下文完整**的事件，可被**无限个下游域**各自消费、各自衍生内容——一条消息到达 → 通知中心弹一条 / 未读徽章+1 / @我聚合流加一条 / 运维审计记一笔 / 摘要 digest 收一条……core 的事件契约不变，下游域可无限加。这是 GOAL §3「加新业务模块 core 一行不改」在**消费侧**的镜像。

## §2 域不是功能，是消费者

你点名的「系统通知 / 业务模块启动 / 艾特与我相关 / 运维」**不是我们要实现的功能**，而是**下游消费者域（consumer domain）**。EventBus 是**通用底座**，每个域是建在它之上、各取所需的独立订阅者：

```
        通用事件底座（helix-core EventSink + 上下文完整信封 + 订阅切片 + catch-up）
                    │  一条 self-describing event 扇出 N 份
   ┌──────────┬──────────┬──────────┬──────────┬──────────┐
 系统通知域   生命周期域   @我/相关域   运维域(下线)  未来 ∞ 域
（弹窗/徽章）（启动/连接） （mention 聚合）（已下线）   （摘要/搜索/…）
```

- core **永远不认识**这些域（GOAL §1.6 硬约束）；
- 每个域只声明「我订阅哪类事件 + 我自己怎么解释 relevance」；
- **「无限内容」= 域可无限加，而 core 事件契约不变**。

## §3 反推出的五类能力契约（缺一个，对应下游域就得回头调我们）

| # | 下游要做的事 | 反推出我们必须提供 | 缺了会怎样 | 现状/落点 |
|---|---|---|---|---|
| C1 | 不回调就知道「发生了什么」 | **自描述、上下文完整**的信封（见 §4 字段表） | 下游拿到事件还得反查 DB/调接口，扇出链断 | 部分（现网透传 raw，已定补字段，见 §4.1） |
| C2 | 自己判断「与我相关」 | 信封**透传 relevance 原始字段**（mention/reply-to/assignee/tenant/actor），**core/im 不下结论** | 下游算不出「关不关我」，或被迫回 core 问 | ⚠️ 见 §4.2 决议 |
| C3 | 各取所需的切片 | **订阅 + 过滤**：下游声明 `type∈{...} ∧ scope=X`，按域分发（对齐现网 `im:__bus__` 单总线 + 信封 + 懒注册） | 每个域收全量自己过滤，性能+耦合双输 | 现网单总线已有，过滤契约待定 |
| C4 | 过程中实时关注 | **live push**：`Effect::Emit` → `EventSink::emit`（fire-and-forget） | —— | ✅ 已有 |
| C5 | 结束后补齐（事后实时关注） | **catch-up / replay**：晚到/掉线的消费者从某 cursor 补齐错过的事件 | 一掉线就永久丢事件，「事后关注」做不到 | ⚠️ 复用 HX-C008 cursor 契约，待接 EventBus |

> C4+C5 = 「会议**过程中** / **结束后**实时关注」两条腿：live（过程中）+ catch-up（结束后），缺一个下游都没法「持续关注与我相关的信息」。

## §4 信封上下文契约（C1/C2 的具体字段）

### §4.1 自描述信封字段（反推自「下游不回调就能干活」）

> 字段是**契约**，必须**版本化**（防 schema 漂移地狱，过会拍板 #5）。core 仍只搬 `DomainEventBytes` 不透明字节；下列结构由 **helix-im 序列化层**保证，core 不解析。

| 字段 | 含义 | 哪个下游域要它 |
|---|---|---|
| `v` | 信封版本 | 全部（演进兼容） |
| `type` | 事件类型（如 `im:post:received` / `im:channel:increment` / `im:connection:established`） | C3 订阅过滤 |
| `actor` | 谁触发（发送者/系统/对端用户 id） | 通知域、@我域、审计域 |
| `target` / `scope` | 作用对象 + 归属（channel_id / tenant_key） | C3 过滤、通知域路由 |
| `ts` | 发生时间（host 注入，确定性） | 全部排序 |
| `event_seq` | 单调序号（catch-up 锚点） | C5 replay |
| `causation_id` | 由哪次操作/帧引发（因果链） | 审计域、调试、「一次操作扇出」溯源 |
| `payload` | 业务原始数据（**含 relevance 原始字段**，见 §4.2） | 各域自取 |

#### §4.1.1 信封补字段决议（已定 · 2026-06-16）

现网 `im:__bus__` 信封缺 `actor` / `event_seq` / `causation_id`，导致「非我才弹」「掉线 catch-up」「扇出溯源」做不了。**拍板：三字段升为信封必填**，规格如下（落地到 `to_effect.rs` emit）：

| 字段 | 类型 | 语义 | 谁来填 | 解锁 |
|---|---|---|---|---|
| `actor` | id（string） | 触发主体：发送者 / 系统 / 对端用户 id；系统事件填保留值 `__system__` | helix-im 序列化时 | 通知域「非我才弹」（`actor != me`）、审计 |
| `event_seq` | u64 单调 | 事件单调序号，**catch-up replay 锚点**；与业务行号解耦（对齐 HX-C008、performance.md「事件流单调 cursor」） | host 注入（确定性，core 不自生成） | C5 掉线补齐、@我流去重键 |
| `causation_id` | id（string，可空） | 由哪次入站帧 / 操作引发（因果链）；同一操作扇出的多事件共享同一 id | helix-im 透传触发帧的 corr/id | 审计、调试、「一次操作引发无限内容」溯源 |

> `actor` / `causation_id` 是**业务字段** → 在 helix-im 填，`grep crates/helix-core` 必须为 0（GOAL §1.6）。`event_seq` 是通用单调原语 → host 注入，core 只搬不解释。

### §4.2 relevance 归属（已拍：**各下游域自己算**）

**决议**：core 与 helix-im **只透传 relevance 的原始字段，绝不算「是否与我相关」的结论**。

- **理由**：与 GOAL §1.6（业务不污染内核）+ §6 方向 A 边界一致——core 可以说「这条 payload 里有 mentionList」，**不能说**「这条@了用户 U，给 U 弹红点」；「@了谁、关不关当前登录用户」是**带 `me` 上下文的业务判断**，只有持有 `me` 的下游域有资格算。
- **硬约束**：信封 `payload` **必须**完整透传判 relevance 所需的原始字段（如 `mentionList` / `urgentPostList` / `reply_to` / `assignee` / `actor` / `tenant_key`），缺字段 = 下游域算不出 = 契约违约。对齐现网 `im:channel:increment` 已透传 `mentionList/urgentPostList` 的现状。
- **代价（明示）**：通知域、@我域、徽章域会各自实现一遍「关不关我」。可接受——换来 core 纯净 + 域解耦；若未来 ≥3 个域重复同一段 relevance 逻辑，**再**抽成一个**下游侧共享库**（仍在业务层，不进 core）。

## §5 下游域 × 最小上下文反推表

> 每行回答 §1 判据：该域只拿事件能否干活；右列 = 它依赖 §4.1 的哪些字段。

| 下游域 | 它要做什么 | 订阅的 `type` | 必需的最小上下文 | relevance 由谁算 |
|---|---|---|---|---|
| **系统通知域** | 弹窗 / 横幅 / 声音 | `im:post:received`、`im:channel:increment` | actor + scope + payload 摘要 | 域自己（按 §4.2） |
| **生命周期域** | 显示连接/模块「启动·就绪·断线」 | `im:connection:established`、`*:lifecycle` | type + connection_id + ts | 不涉及（全局态） |
| **@我/与我相关域** | 聚合「@我/回复我/指派我」成一条流 + 红点 | `im:channel:increment`、`im:post:received` | payload.mentionList / reply_to / assignee + actor + scope | **域自己**（持 `me`） |
| **未读徽章域** | per-channel / 全局未读计数 | `im:channel:increment` | scope + payload 增量计数 | 域自己 |
| ~~运维域~~ 🚫 已下线 | ops 播报 / 审计留痕 | ~~`im:ops:*`~~（移出 roadmap，2026-06-16） | —— | 现存运维场景走 interceptor/UpdateService |
| **同步状态域** | 「同步到哪了」（**注意：这是 REP 不是 PUB**） | —— | 见第二部分 §9（UI 主动查询） | —— |

> 末行警示：**同步进度「播报」走 PUB Emit，「UI 主动问当前进度」走 REP**——别混（判据见 §0、§10）。

## §6 与现有机制的关系（PUB 侧不重复造轮子）

- **PUB（本契约主体）**：`Effect::Emit` → `EventSink::emit`，单向广播，无回音。§3 C1–C5、§5 绝大多数域都在这条线。
- **REP（第二部分）**：需要「一问一答配对才能推进」的，走 `Request`/`Reply`+corr，**不要**塞进 PUB 信封。
- **批次边界 = 一个 step（HX-C007）**：一个 step 的所有 Emit 攒一批整体过 UI 边界，禁逐条；Persist/Http/Send 在 driver 内消化、不过 UI 边界。
- **catch-up 复用 HX-C008**：C5 的 replay 锚点 = §4.1 `event_seq`，对齐 sync cursor「锚 nextSeq、续拉看 len≥500」契约，**不新造 cursor 机器**。
- **GOAL §6 方向 A 关系**：本契约是「事件通信」线；未来若 core 吐 ViewModel（方向 A），ViewModel 走 `Effect::Render` 是**另一条线**，与 EventBus 并列，不混。

---

# 第二部分 · REP —— 双向请求-应答通道

## §7 机制总览：两个方向，复用同一套 corr 机器

sans-IO 内核不能 `await`，所以 REP 永远是**异步配对**：请求作为 `Effect` 现在发出，回复作为 `Tick` 将来到达，靠 corr 配对。**不是阻塞式 call-return。**

```
方向① core 主动问宿主（core → host → core）
   模块吐 Effect::Request{corr,kind,payload}
        └─ driver 转交宿主(UI/FFI host/JS) ─┐
                                            ▼ 宿主处理
   模块 handle ◄── Tick::PortReply{corr,outcome} ◄── corr_map 定向回投（O(1)）

方向② 宿主主动问 core（host → core → host）
   宿主发 Tick::Command{corr_h,...}
        └─ accepts() 路由到模块 ─┐
                                ▼ 模块同步算答案
   宿主侧配对 ◄── Effect::Reply{corr_h,payload} ◄── driver 转交宿主
                  （corr_h 由宿主分配，**不进** core 的 corr_map）
```

| | 方向① core→host→core | 方向② host→core→host |
|---|---|---|
| 发起 | 模块吐 `Effect::Request{corr}` | 宿主发 `Tick::Command{corr_h}` |
| 路由 | corr 由 core 分配器发放，登记进 `corr_map` | `Command` 走 `accepts()` 线性路由 |
| 回复 | `Tick::PortReply{corr}` → `corr_map.consume` 定向回投 | 模块吐 `Effect::Reply{corr_h}` → driver 投回宿主 |
| corr 归属 | **core 分配器**（`corr_map` 持有） | **宿主分配**（宿主自己的 pending 表持有） |

**corr 命名空间隔离（铁律）**：进 `corr_map` 的 corr **只能**来自 core 主动发起的 Effect（`Persist`/`Http`/`HttpFire`/`Request`）；方向②宿主自造的 `corr_h` **绝不进** `corr_map`，由宿主自己配对。两个 corr 空间天然不串台。（已被 `unknown_corr_reply_silently_ignored` 测试钉死。）

> 🆕 **corr_map 统一收口（refactor B，2026-06-17 已落地）**：原本 5 张 `corr_to_*` HashMap 已收敛为单一 `corr_map: HashMap<Correlation, CorrelationContext>`，`handle_port_reply` 在一个 `match` 上收口所有回投（Persist/Http/HttpFire/Request 共用同一张表 + 同一回投路径）。REP 方向①直接复用这套统一的 corr_map + `Tick::PortReply`，零新机制。

## §8 方向①场景：入群邀请待用户决策

最纯粹的 REP——core 状态机走到一个**必须由人拍板**的分叉点，不拿到用户的「同意/拒绝」无法继续，且要容忍用户**不点**（超时兜底）。

### §8.1 时序

```
① 服务端推来"入群邀请"帧
   Tick::Inbound(invite_frame)
        ▼ helix-im 解析 → 状态机进入 PendingInviteDecision
② 模块向宿主提问 + 同时 arm 超时
   out.push(Effect::Request{ corr=C1, kind="invite_decision", payload={inviter,channel,...} })
   out.push(Effect::ScheduleTimer{ id=T1, after_ms=30_000 })   // 30s 不点按默认处理
        ▼ corr_map: C1 → 本模块；timer_map: T1 → 本模块
③ 宿主弹 UI，用户点"同意"
   driver 把决策包成 Tick::PortReply{ corr=C1, outcome=Ok("accept") }
        ▼ corr_map.consume(C1) → 定向回投本模块
④ 模块据回复推进：发接受入群请求 + 取消超时
   out.push(Effect::Http{ corr=C2, req=accept_invite })   // 后续 I/O 走 Http corr 回路
   out.push(Effect::CancelTimer{ id=T1 })
```

### §8.2 三条分支（每个 REP 场景的必答题：回复迟到/不来怎么办）

| 分支 | 触发 | 模块动作 |
|---|---|---|
| 用户同意 | `PortReply{C1, Ok("accept")}` | 发 `Http{accept}` + `CancelTimer{T1}` |
| 用户拒绝 | `PortReply{C1, Ok("decline")}` | 清理 PendingInviteDecision + `CancelTimer{T1}` |
| **用户不点（超时）** | `Tick::Timer{T1}` 先到 | 走默认（丢弃邀请 / 标记待办）；此后 `C1` 的迟到回复落到 `corr_map` 已无 C1 → **静默忽略** |

> 关键：超时分支让 REP 不会把状态机永久卡死。迟到回复的静默容忍由 core 保证（`corr_map.consume` 未命中即 `warn` + drop），模块无需额外防御。
> 注：上表的超时 timer T1 是 one-shot——其 timeout handler 分支必须在产出业务 Effect 的同时补 `Effect::CancelTimer(self)`（core 无 one-shot 语义，否则 timer_map 慢泄漏，见 sync-v2 设计 §8.2）。

### §8.3 落地待办（方向①收尾）

1. 在 driver 定义 `HostRequester` port：`Effect::Request` → 转交宿主 → 宿主回复包成 `Tick::PortReply{corr}` 经 reply_tx 回灌（替换 native `engine_loop` 当前的 stub 臂）。两端各实现一次（native AppHandle、FFI 回调）。
2. helix-im 加 `PendingInviteDecision` 状态 + `alloc_corr()` 发起 + 三分支 handle + 回放测试。

## §9 方向②场景：UI 查询同步进度

最干净的 host→core→host——UI 按需问「现在同步到哪了？」，core 从**内存态**即时返回快照（纯内存，无存储绕行），且多次查询要各自配对到自己的回答。

### §9.1 时序

```
① 用户点"同步中"图标，UI 想看详情
   宿主分配 corr_h=H1（宿主自己的 pending 表）
   Tick::Command{ corr=Some(H1), name="query_sync_progress", payload={} }
        ▼ accepts() 路由到 helix-im
② 模块同步读内存态，立即吐回复
   let snap = self.sync_session.progress();  // {phase, synced_seq, target_seq}
   out.push(Effect::Reply{ corr=H1, payload=serialize(snap) })
        ▼ driver 把 Reply 投回宿主（corr_h 不进 corr_map）
③ 宿主按 H1 配对，渲染进度条
```

### §9.2 需要新增的类型（方向②落地）

```rust
// tick.rs：Command 增加可选 corr（宿主分配；None = 无需回复的纯命令）
pub struct AppCommand {
    pub name: Cow<'static, str>,
    pub payload: bytes::Bytes,
    pub corr: Option<Correlation>,   // ← 新增：方向② 请求标识，宿主分配
}

// effect.rs：方向② 回复端（与 Emit 区别：带 corr，定向回宿主而非广播）
Effect::Reply {
    corr: Correlation,               // 回显宿主分配的 corr_h，宿主据此配对
    payload: bytes::Bytes,
},
```

`engine.rs`：`Effect::Reply` **不进** `corr_map`（它不期待 core 侧回投，corr 归宿主）。driver 把 `Effect::Reply` 转发给宿主的 pending-query 配对表。

### §9.3 为什么必须 REP（不能用 Emit 替代）

- 多个查询可能并发在途（用户连点 / 多窗口），回答必须配对到**是哪次查询**——这正是 corr 的作用。
- 同步进度若改成 core 主动 `Emit` 推送，UI 只能被动等下一次推送，无法「此刻就要答案」。on-demand 查询 = REP。

> 注：若查询答案需读**存储**（如「拉某 channel 历史一页」），则方向②会在内部触发一次**方向①式的 I/O REP**（`Effect::Persist{Scan,corr}` → `PortReply` → 再 `Effect::Reply{corr_h}` 回宿主）——两条 REP 组合，互不串台（一个 corr 在 corr_map，一个 corr_h 在宿主表）。本场景选纯内存的同步进度，避免存储绕行，做最小示范。

## §10 易误判清单（看着像 REP，其实不是）

| 场景 | 真相 | 该用什么 |
|---|---|---|
| 发消息「发送成功」 | 存储/网络端口的回音，非宿主线 | 现成 `Persist{corr}`/`Http{corr}` → `PortReply`，**别新造 Request** |
| 新消息到达、艾特我、运维播报 | 没人要回音 | 单向 `Emit` |
| 已读回执、typing、presence 上下线 | 状态广播 | 单向 `Emit` |
| 同步进度**播报** | core 主动推 | 单向 `Emit`（只有 UI **主动问**才是方向② REP，见 §9） |
| 消息「送达对端」ack | 异步事件流，不与某次本地请求配对 | `Emit` / 单调 cursor |

## §11 当前实现状态

| 部件 | 状态 |
|---|---|
| `Effect::Request{corr,kind,payload}`（方向①发起端） | ✅ 已落 `effect.rs` |
| `engine.rs` corr_map 登记 `Request` | ✅ 已落（并入统一 corr_map 登记臂，refactor B 后单 `match` 收口） |
| `Tick::PortReply` 回路（方向①回投） | ✅ 复用现成，零新机制 |
| 回放测试（方向① core 侧闭环 4 例） | ✅ `tests/request_reply_replay.rs` 全过 |
| native driver `Effect::Request` 执行 | ⚠️ **deferred stub**（仅 `warn!`，待接 `HostRequester` port） |
| 方向② `Command{corr}` + `Effect::Reply` | ❌ 未实现（本文档定设计） |

## §12 确定性与一致性

- 两个方向都是「发请求 Effect → 收回复 Tick」，core 内零 await、零 I/O，`step()` 严格同步 → **可回放、可剧本测试**（方向①已验证，方向②落地时同样配回放测试）。
- 与已拍板结论一致：`Effect` 仍是**单一 enum**，新增 `Request`/`Reply` 是 additive 变体，四能力解耦保持**行为级**（模块只 emit 它需要的 Effect），非编译级裁剪。
- corr 由分配器单调发放（`effect.rs` 已用 `pub(crate)` 锁死构造器），方向②宿主侧 corr 自管，两空间隔离。

---

## §13 落地 checklist

PUB（下游能力契约）：
- [x] §4.1 信封补字段 `actor`/`event_seq`/`causation_id` **已定必填**（§4.1.1，2026-06-16）；落地到 `to_effect.rs` emit 待办
- [ ] §4.2 relevance「域自己算」写入硬约束 + payload 透传字段清单（防漏字段）
- [x] ~~ADR：运维消息 `im:ops:*`~~ → **运维域已下线**（移出 roadmap，2026-06-16；重启需先立 ADR）
- [ ] C3 订阅/过滤契约：下游声明 `type ∧ scope` 的格式（沿用单总线还是分 topic）
- [ ] C5 catch-up：EventBus 事件流接 `event_seq` replay 的最小设计（复用 HX-C008）
- [ ] 每个域一份「最小上下文清单」补全 §5 表（新域加入时强制过判据）

REP（双向通道）：
- [x] 方向① `Effect::Request` + corr_map 登记 + core 回放测试（已完成）
- [ ] 方向① `HostRequester` port（两端）替换 native stub 臂
- [ ] 方向① helix-im「入群邀请待决策」状态机 + 超时三分支 + 回放测试
- [ ] 方向② `AppCommand.corr` + `Effect::Reply` + `engine.rs`（Reply 不进 corr_map）
- [ ] 方向② driver 宿主 pending-query 配对表（两端）
- [ ] 方向② helix-im「查询同步进度」handle + 回放测试
- [ ] `Effect` 公开类型面变更过 `cargo public-api` 闸门（过会拍板 #2）

---

## §14 一句话收尾

EventBus 的价值不在「我们 emit 了什么」，在「**下游不回头问我们就能干活**」。PUB 把信封做到**上下文完整 + relevance 原始字段透传 + 可订阅 + 可补齐**，core 一条事件就能被无限下游域扇成无限内容；REP 用同一套 corr 机器补上「必须一问一答才能推进」的那少数几条线（方向① core 问宿主 / 方向② 宿主问 core），两条线判据清晰、互不串台。
