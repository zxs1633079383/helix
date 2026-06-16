# Helix EventBus · REP 双向通道设计（方向①②各一场景）

> 归属：MVP1（Rust-Core 与二端 Driver 骨架打底）的 EventBus 能力补全。
> 机制权威源对齐 [架构设计](./helix-core-im-架构设计.md)；与[已拍板结论](./README.md#-已拍板结论七条--各文档须与此一致) §3「`Effect` 单一 enum、四能力行为级解耦」一致。
> 代码落点：`crates/helix-core/src/{effect.rs,tick.rs,engine.rs}`；回放测试 `crates/helix-core/tests/request_reply_replay.rs`。

---

## 0. 一句话与判据

EventBus 有两种语义：

- **PUB（单向广播）**：`Effect::Emit` → `EventSink::emit()`，fire-and-forget，发了就走，无回音。
- **REP（请求-应答，双向）**：发起方发一个请求，**必须**拿到一个与之一一对应的回答才能推进状态机；靠 `Correlation`（corr）把"问"和"答"配起来。

**判据（设计每个事件前先过）**：
> 「发出后，发起方是否必须拿到一个**与这次请求一一对应**的回答，才能推进自己的状态机？」
> 是 → REP；否 → PUB。
> 且：如果那个回答本来就来自**存储 / HTTP 端口**，用现成的 `Persist{corr}` / `Http{corr}` 回路，**不要新造 `Request`**——REP（`Effect::Request`）只服务 "core ↔ 宿主/UI" 这条线。

---

## 1. 机制总览：两个方向，复用同一套 corr 机器

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

**corr 命名空间隔离（铁律）**：进 `corr_map` 的 corr **只能**来自 core 主动发起的 Effect（`Persist`/`Http`/`Request`）；方向②宿主自造的 `corr_h` **绝不进** `corr_map`，由宿主自己配对。两个 corr 空间天然不串台。（已被 `unknown_corr_reply_silently_ignored` 测试钉死。）

---

## 2. 当前实现状态

| 部件 | 状态 |
|---|---|
| `Effect::Request{corr,kind,payload}`（方向①发起端） | ✅ 已落 `effect.rs` |
| `engine.rs` corr_map 登记 `Request` | ✅ 已落（并入 `Persist｜Http｜Request` 登记臂） |
| `Tick::PortReply` 回路（方向①回投） | ✅ 复用现成，零新机制 |
| 回放测试（方向① core 侧闭环 4 例） | ✅ `tests/request_reply_replay.rs` 全过 |
| native driver `Effect::Request` 执行 | ⚠️ **deferred stub**（仅 `warn!`，待接 `HostRequester` port） |
| 方向② `Command{corr}` + `Effect::Reply` | ❌ 未实现（本文档定设计） |

---

## 3. 方向①选定场景：**入群邀请待用户决策**

最纯粹的 REP——core 状态机走到一个**必须由人拍板**的分叉点，不拿到用户的「同意/拒绝」无法继续，且要容忍用户**不点**（超时兜底）。

### 3.1 时序

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

### 3.2 三条分支（每个 REP 场景的必答题：回复迟到/不来怎么办）

| 分支 | 触发 | 模块动作 |
|---|---|---|
| 用户同意 | `PortReply{C1, Ok("accept")}` | 发 `Http{accept}` + `CancelTimer{T1}` |
| 用户拒绝 | `PortReply{C1, Ok("decline")}` | 清理 PendingInviteDecision + `CancelTimer{T1}` |
| **用户不点（超时）** | `Tick::Timer{T1}` 先到 | 走默认（丢弃邀请 / 标记待办）；此后 `C1` 的迟到回复落到 `corr_map` 已无 C1 → **静默忽略** |

> 关键：超时分支让 REP 不会把状态机永久卡死。迟到回复的静默容忍由 core 保证（`corr_map.consume` 未命中即 `warn` + drop），模块无需额外防御。

### 3.3 落地待办（方向①收尾）

1. 在 driver 定义 `HostRequester` port：`Effect::Request` → 转交宿主 → 宿主回复包成 `Tick::PortReply{corr}` 经 reply_tx 回灌（替换 native `engine_loop.rs` 当前的 stub 臂）。三端各实现一次（native AppHandle、FFI 回调、wasm JS callback）。
2. helix-im 加 `PendingInviteDecision` 状态 + `alloc_corr()` 发起 + 三分支 handle + 回放测试。

---

## 4. 方向②选定场景：**UI 查询同步进度**

最干净的 host→core→host——UI 按需问"现在同步到哪了？"，core 从**内存态**即时返回快照（纯内存，无存储绕行），且多次查询要各自配对到自己的回答。

### 4.1 时序

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

### 4.2 需要新增的类型（方向②落地）

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

### 4.3 为什么必须 REP（不能用 Emit 替代）

- 多个查询可能并发在途（用户连点 / 多窗口），回答必须配对到**是哪次查询**——这正是 corr 的作用。
- 同步进度若改成 core 主动 `Emit` 推送，UI 只能被动等下一次推送，无法"此刻就要答案"。on-demand 查询 = REP。

> 注：若查询答案需读**存储**（如"拉某 channel 历史一页"），则方向②会在内部触发一次**方向①式的 I/O REP**（`Effect::Persist{Scan,corr}` → `PortReply` → 再 `Effect::Reply{corr_h}` 回宿主）——两条 REP 组合，互不串台（一个 corr 在 corr_map，一个 corr_h 在宿主表）。本场景选纯内存的同步进度，避免存储绕行，做最小示范。

---

## 5. ⚠️ 易误判清单（看着像 REP，其实不是）

| 场景 | 真相 | 该用什么 |
|---|---|---|
| 发消息"发送成功" | 存储/网络端口的回音，非宿主线 | 现成 `Persist{corr}`/`Http{corr}` → `PortReply`，**别新造 Request** |
| 新消息到达、艾特我、运维播报 | 没人要回音 | 单向 `Emit` |
| 已读回执、typing、presence 上下线 | 状态广播 | 单向 `Emit` |
| 同步进度**播报** | core 主动推 | 单向 `Emit`（只有 UI **主动问**才是方向② REP，见 §4） |
| 消息"送达对端"ack | 异步事件流，不与某次本地请求配对 | `Emit` / 单调 cursor |

---

## 6. 确定性与一致性

- 两个方向都是"发请求 Effect → 收回复 Tick"，core 内零 await、零 I/O，`step()` 严格同步 → **可回放、可剧本测试**（方向①已验证，方向②落地时同样配回放测试）。
- 与[已拍板结论](./README.md) §3 一致：`Effect` 仍是**单一 enum**，新增 `Request`/`Reply` 是 additive 变体，四能力解耦保持**行为级**（模块只 emit 它需要的 Effect），非编译级裁剪。
- corr 由分配器单调发放（`effect.rs` 已用 `pub(crate)` 锁死构造器），方向②宿主侧 corr 自管，两空间隔离。

---

## 7. 落地 checklist

- [x] 方向① `Effect::Request` + corr_map 登记 + core 回放测试（已完成）
- [ ] 方向① `HostRequester` port（三端）替换 native stub 臂
- [ ] 方向① helix-im「入群邀请待决策」状态机 + 超时三分支 + 回放测试
- [ ] 方向② `AppCommand.corr` + `Effect::Reply` + `engine.rs`（Reply 不进 corr_map）
- [ ] 方向② driver 宿主 pending-query 配对表（三端）
- [ ] 方向② helix-im「查询同步进度」handle + 回放测试
- [ ] `Effect` 公开类型面变更过 `cargo public-api` 闸门（结论 §2）
