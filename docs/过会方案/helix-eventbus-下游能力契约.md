# Helix EventBus · 下游能力契约（反推驱动设计）

> 归属：MVP1 EventBus 能力定线——回答「**下游要建得起来，我们的事件必须保证哪些上下文与能力**」。
> 机制权威源对齐 [架构设计](./helix-core-im-架构设计.md) §3.1；PUB/REP 语义对齐 [REP 双向通道设计](./helix-eventbus-rep-双向通道设计.md)；红线对齐 [GOAL](../GOAL.md) §1.6（业务不污染内核）/ §6（抽象高度边界）。
> 本文是**设计契约**，不碰代码。落点类型在 `crates/helix-core/src/{effect.rs,ports/event_sink.rs}` 与 helix-im 信封序列化层。

---

## §0 一句话与判据（设计驱动方向的反转）

**普通做法（生产者驱动）**：我们 emit 什么 → 下游爱用不用。
**本契约（消费者反推）**：先问「下游某个域要拿这条事件去干什么、缺哪些上下文才**不必回头调我们**」→ 反推出事件**必须自带**什么能力与上下文 → 我们吐出去的每条事件对下游**自解释、有价值**。

> **反推判据（设计每条事件前先过）**：
> 「一个下游域，只拿到这条事件本身（不查 DB、不调接口、不回问 core），能不能完成它的工作（弹通知 / 加徽章 / 进 @我流 / 记审计）？」
> 能 → 上下文够；不能 → 缺的字段就是本契约要补的。

**为什么这能"一次操作引发无限内容"**：一条**上下文完整**的事件，可被**无限个下游域**各自消费、各自衍生内容——一条消息到达 → 通知中心弹一条 / 未读徽章+1 / @我聚合流加一条 / 运维审计记一笔 / 摘要 digest 收一条……core 的事件契约不变，下游域可无限加。这就是 GOAL §3「加新业务模块 core 一行不改」在**消费侧**的镜像。

---

## §1 域不是功能，是消费者（摆正位置）

你点名的「系统通知 / 业务模块启动 / 艾特与我相关 / 运维」**不是我们要实现的功能**，而是**下游消费者域（consumer domain）**。EventBus 是**通用底座**，每个域是建在它之上、各取所需的独立订阅者：

```
        通用事件底座（helix-core EventSink + 上下文完整信封 + 订阅切片 + catch-up）
                    │  一条 self-describing event 扇出 N 份
   ┌──────────┬──────────┬──────────┬──────────┬──────────┐
 系统通知域   生命周期域   @我/相关域   运维域(待拍)  未来 ∞ 域
（弹窗/徽章）（启动/连接） （mention 聚合）（ops 审计）  （摘要/搜索/…）
```

- core **永远不认识**这些域（GOAL §1.6 硬约束）；
- 每个域只声明「我订阅哪类事件 + 我自己怎么解释 relevance」；
- **"无限内容" = 域可无限加，而 core 事件契约不变**。

---

## §2 反推出的五类能力契约（缺一个，对应下游域就得回头调我们）

| # | 下游要做的事 | 反推出我们必须提供 | 缺了会怎样 | 现状/落点 |
|---|---|---|---|---|
| C1 | 不回调就知道"发生了什么" | **自描述、上下文完整**的信封（见 §3 字段表） | 下游拿到事件还得反查 DB/调接口，扇出链断 | 部分（现网透传 raw，未版本化字段表） |
| C2 | 自己判断"与我相关" | 信封**透传 relevance 原始字段**（mention/reply-to/assignee/tenant/actor），**core/im 不下结论** | 下游算不出"关不关我"，或被迫回 core 问 | ⚠️ 见 §3.2 决议 |
| C3 | 各取所需的切片 | **订阅 + 过滤**：下游声明 `type∈{...} ∧ scope=X`，按域分发（对齐现网 `im:__bus__` 单总线 + 信封 + 懒注册） | 每个域收全量自己过滤，性能+耦合双输 | 现网单总线已有，过滤契约待定 |
| C4 | 过程中实时关注 | **live push**：`Effect::Emit` → `EventSink::emit`（fire-and-forget） | —— | ✅ 已有 |
| C5 | 结束后补齐（事后实时关注） | **catch-up / replay**：晚到/掉线的消费者从某 cursor 补齐错过的事件 | 一掉线就永久丢事件，"事后关注"做不到 | ⚠️ 复用 HX-C008 cursor 契约，待接 EventBus |

> C4+C5 = 你说的「会议**过程中** / **结束后**实时关注」两条腿：live（过程中）+ catch-up（结束后），缺一个下游都没法"持续关注与我相关的信息"。

---

## §3 信封上下文契约（C1/C2 的具体字段）

### §3.1 自描述信封字段（反推自"下游不回调就能干活"）

> 字段是**契约**，必须**版本化**（防 schema 漂移地狱，纪要拍板 #5）。core 仍只搬 `DomainEventBytes` 不透明字节；下列结构由 **helix-im 序列化层**保证，core 不解析。

| 字段 | 含义 | 哪个下游域要它 |
|---|---|---|
| `v` | 信封版本 | 全部（演进兼容） |
| `type` | 事件类型（如 `im:post:received` / `im:channel:increment` / `im:conn:established`） | C3 订阅过滤 |
| `actor` | 谁触发（发送者/系统/对端用户 id） | 通知域、@我域、审计域 |
| `target` / `scope` | 作用对象 + 归属（channel_id / tenant_key） | C3 过滤、通知域路由 |
| `ts` | 发生时间（host 注入，确定性） | 全部排序 |
| `event_seq` | 单调序号（catch-up 锚点） | C5 replay |
| `causation_id` | 由哪次操作/帧引发（因果链） | 审计域、调试、"一次操作扇出"溯源 |
| `payload` | 业务原始数据（**含 relevance 原始字段**，见 §3.2） | 各域自取 |

### §3.2 relevance 归属（已拍：**各下游域自己算**）

**决议**：core 与 helix-im **只透传 relevance 的原始字段，绝不算"是否与我相关"的结论**。

- **理由**：与 GOAL §1.6（业务不污染内核）+ §6 方向 A 边界一致——core 可以说「这条 payload 里有 mentionList」，**不能说**「这条@了用户 U，给 U 弹红点」；"@了谁、关不关当前登录用户"是**带 `me` 上下文的业务判断**，只有持有 `me` 的下游域有资格算。
- **硬约束**：信封 `payload` **必须**完整透传判 relevance 所需的原始字段（如 `mentionList` / `urgentPostList` / `reply_to` / `assignee` / `actor` / `tenant_key`），缺字段 = 下游域算不出 = 契约违约。对齐现网 `im:channel:increment` 已透传 `mentionList/urgentPostList` 的现状。
- **代价（明示）**：通知域、@我域、徽章域会各自实现一遍"关不关我"。可接受——换来 core 纯净 + 域解耦；若未来 ≥3 个域重复同一段 relevance 逻辑，**再**抽成一个**下游侧共享库**（仍在业务层，不进 core）。

---

## §4 下游域 × 最小上下文反推表（逐域过判据）

> 每行回答 §0 判据：该域只拿事件能否干活；右列 = 它依赖 §3.1 的哪些字段。

| 下游域 | 它要做什么 | 订阅的 `type` | 必需的最小上下文 | relevance 由谁算 |
|---|---|---|---|---|
| **系统通知域** | 弹窗 / 横幅 / 声音 | `im:post:received`、`im:channel:increment` | actor + scope + payload 摘要 | 域自己（按 §3.2） |
| **生命周期域** | 显示连接/模块"启动·就绪·断线" | `im:conn:established`、`*:lifecycle` | type + connection_id + ts | 不涉及（全局态） |
| **@我/与我相关域** | 聚合"@我/回复我/指派我"成一条流 + 红点 | `im:channel:increment`、`im:post:received` | payload.mentionList / reply_to / assignee + actor + scope | **域自己**（持 `me`） |
| **未读徽章域** | per-channel / 全局未读计数 | `im:channel:increment` | scope + payload 增量计数 | 域自己 |
| **运维域（待拍）** | ops 播报 / 审计留痕 | `im:ops:*`（**未定**） | actor + causation_id + ts + 原始帧 | 不涉及 |
| **同步状态域** | "同步到哪了"（**注意：这是 REP 不是 PUB**） | —— | 见 REP 文档 §4（UI 主动查询） | —— |

> 末行警示：**同步进度"播报"走 PUB Emit，"UI 主动问当前进度"走 REP**——别混（判据见 REP 文档 §0/§5）。

---

## §5 与现有机制的关系（不重复造轮子）

- **PUB（本契约主体）**：`Effect::Emit` → `EventSink::emit`，单向广播，无回音。§2 C1–C5、§4 绝大多数域都在这条线。
- **REP（不在本契约）**：需要"一问一答配对才能推进"的，走 [REP 文档](./helix-eventbus-rep-双向通道设计.md) 的 `Request`/`Reply`+corr，**不要**塞进 PUB 信封。
- **批次边界 = 一个 step（HX-C007）**：一个 step 的所有 Emit 攒一批整体过 UI 边界，禁逐条；Persist/Http/Send 在 driver 内消化、不过 UI 边界。
- **catch-up 复用 HX-C008**：C5 的 replay 锚点 = §3.1 `event_seq`，对齐 sync cursor「锚 nextSeq、续拉看 len≥500」契约，**不新造 cursor 机器**。
- **GOAL §6 方向 A 关系**：本契约是"事件通信"线；未来若 core 吐 ViewModel（方向 A），ViewModel 走 `Effect::Render` 是**另一条线**，与 EventBus 并列，不混。

---

## §6 待拍 / 落地 checklist

- [ ] §3.1 信封字段表定稿 + 版本化（与现网 `im:__bus__` envelope 对齐字段名）
- [ ] §3.2 relevance「域自己算」写入硬约束 + payload 透传字段清单（防漏字段）
- [ ] **ADR：运维消息 `im:ops:*`**（目前 ADR-008 标"待拍"）——定它走 PUB 还是 REP、信封含哪些 ops 上下文
- [ ] C3 订阅/过滤契约：下游声明 `type ∧ scope` 的格式（沿用单总线还是分 topic）
- [ ] C5 catch-up：EventBus 事件流接 `event_seq` replay 的最小设计（复用 HX-C008）
- [ ] 每个域一份"最小上下文清单"补全 §4 表（新域加入时强制过判据）

---

## §7 一句话收尾

EventBus 的价值不在"我们 emit 了什么"，在"**下游不回头问我们就能干活**"。把信封做到**上下文完整 + relevance 原始字段透传 + 可订阅 + 可补齐**，core 一条事件就能被无限下游域扇成无限内容——这就是老大要的"反推"。
