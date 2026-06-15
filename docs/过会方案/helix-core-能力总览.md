# Helix Core 能力总览（站立起来后它会什么）

> 一页纸 · 2026-06-10 · 配合 [架构设计](./helix-core-im-架构设计.md) 使用，评委问「这核到底能干嘛」翻这页。
> **一句话**：站立起来的 core = 一台「确定性事件循环 + 5 类可插拔 I/O 端口」的客户端运行时——它自己什么业务都不会，但任何「在线推送 + 离线同步 + 本地存储」形态的业务装上去，自动获得三端一致、可回放、可灰度的工程性质。

---

## 一、内核本体能力（helix-core 自身，零依赖即具备）

1. **纯状态机执行壳**：`step(Tick, now_ms) → &[Effect]`——喂事件、吐 I/O 意图，严格同步、零 I/O、单线程可跑。这是核的全部「运动方式」。
2. **执行器无关的协程调度器**：corr 机制（吐 `Effect{corr}` → 登记 corr_map → `PortReply{corr}` 定向跳回）把异步 I/O 变成回调路由——**不写一行 async fn 就拥有异步能力**，tokio / 浏览器事件循环 / Dart isolate 都能驱动它。
3. **多模块宿主**：`register()` 挂任意业务模块；入站按 `accepts()` 排他路由（first-match wins）、I/O 回报按 corr 身份路由——模块 A 的回报绝不串给模块 B，**隔离是算法不是纪律**。
4. **定时器调度**：ScheduleTimer / CancelTimer，与 corr 共用 u64 命名空间，跨模块零碰撞。
5. **确定性可回放**：时间 / 随机 / ID 全由 host 注入——同一份 Tick 序列永远吐同一份 Effect 序列。
6. **通用单调写原语**：`monotonic_upsert`（MAX guard）——cursor、心跳、任何只进不退的计量都能用，类比 CAS。
7. **Effect 批处理边界**：一次 step = 一批，跨 wasm/FFI 边界整批传（HX-C007）。
8. **5 类 Port · 7 个 trait 对外契约**：Transport / Storage / HttpRequester / EventSink / Clock·IdSource·Random——内核与外界的全部对话方式。

## 二、接上三套 driver 后获得的平台能力（4 能力 × 3 端 = 12 格全亮）

| 能力 | native（Tauri 桌面） | wasm（浏览器） | FFI（移动） |
|---|---|---|---|
| **WS 实时** | tokio-tungstenite + 重连退避心跳 | web-sys WebSocket（Worker 内） | host 帧注入 |
| **HTTP** | reqwest 连接池 + header 热注入 | gloo-net fetch | host 回调 |
| **存储** | sqlx SQLite 三池 WAL，O(1) 热路径 | sqlite-wasm-rs + OPFS（单连接） | rusqlite + spawn_blocking |
| **事件总线** | broadcast → `im:__bus__` | postMessage 批量 | Dart SendPort 流 |

关键：这 12 格**模块只写一次**——模块吐的是 Effect，不知道也不关心落在哪一格。

## 三、派生工程能力（站立后真正值钱的）

1. **一份业务逻辑跑三端**：M 业务 × P 平台从 M×P 份代码塌成 M+P——项目的存在理由。
2. **业务状态机可单测可回放**：不起网络、不起数据库，喂剧本 Tick 断言 Effect 就能测全部 IM 逻辑；线上 bug 报告 = 一段可重放的 Tick 序列。现网 `handlers/post.rs` 1300 行混着 spawn/Mutex 的逻辑第一次变成可测纯函数。
3. **加业务零改内核**：IM → 会议 → 审计上报，新模块 = 实现 Module + register 一行（架构设计 §六已用两个形状完全不同的模块论证）。
4. **换引擎不重写客户端**：对现网 cses-client 是灰度切换——IPC 四条接缝冻结，Angular 一行不改，秒级回退（架构设计 §4.4.1）。
5. **离线优先应用基座**：增量同步 cursor、乐观 UI 对账、断线补漏做在 Rust-im，但 corr / timer / monotonic_upsert 原语让后续任何模块复用同款模式。

## 四、故意不具备的能力（边界，答辩时主动说防钓鱼）

- **不做 UI / 渲染**——吐 Emit 事件到边界为止（所以只取 Dioxus 的 core 经验、不要它的渲染层）。
- **不做并发**——并发全在 driver 边缘（native 按 channel spawn、wasm 天然串行），核永远单线程可跑：这是三栖的前提不是缺陷。
- **不做请求-响应查询**——读旁路直连 Storage；硬塞查询进内核会把 corr_map 变成 RPC 框架。
- **不认识任何业务**——`grep channel/message/temporaryId crates/helix-core = 0`（2026-06-10 实测通过）。
- **不做服务端**——客户端内核；服务端契约（event_seq / deadQueue / sync 协议）是它的输入假设。

## 五、当前骨架 vs 站立后（诚实差距表）

| 层 | 现状（M0 骨架） | 站立判据 |
|---|---|---|
| 类型契约（Tick/Effect/7 trait/Module） | ✅ 已立，三栖编译通过、零依赖实测通过 | — |
| ExecutionShell（step / corr_map / timer_map） | ✅ 骨架可跑 | proptest 回放测试钉死（M1，待办 D4 前置） |
| 三套 driver 方法体 | native 部分已落地 / wasm·ffi `todo!()` 占位 | M1-M2 逐端落地；Storage `scan` C3 已销账（native 实现，wasm/ffi 随端） |
| Rust-im 业务状态机 | Channel/GateState 骨架，flush 方法体为空 | M2 两 phase（基础收发 → 完整增量同步） |
| 现网灰度接入 | 设计完成（§4.4.1 四接缝） | M4 Tauri 第一站换引擎验收 |

> 站立的最低标准：**剧本化 Tick 回放测试绿 + 三栖 cargo check 绿 + native 端到端跑通 UC-1**——到这一步，上表「二、三」节的能力全部从设计变成事实。
