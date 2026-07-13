# helix-driver-native — 设计图索引

> PC / Tauri 端 driver（平台驱动层）的事件泵与背压模型流程图。
> 真源 = `crates/helix-driver-native/src/`（1981 行）；本目录是从代码沉淀的 mermaid 设计图，**代码改了请同步更新**。
> 契约真源仍是 `crates/helix-driver-native/AGENTS.md` 的 engine_loop 五不变量，本图只是其可视化。

## 这个 crate 是什么

`helix-driver-native` 是 helix 三层架构里**平台驱动层**的 PC/Tauri 实现壳：用 tokio 驱动 `helix-core` 的 sans-IO `ExecutionShell`。
**`tokio::spawn` 全部在此层**（core 保持零运行时依赖）——core 只吐 `&[Effect]`、喂 `Tick`，所有异步协调（WS recv / HTTP 响应 / timer fired / AppCommand）在此汇入 tokio mpsc 统一入站。

WS 协议 / HTTP / SQLite 的真正执行已下沉 `helix-driver-host`（ADR-007/010 双端共享），native 各模块（transport/http/storage）是薄壳 + 生命周期接线。

## 图清单

| 文档 / 图 | 一句话 | 覆盖源码 |
|---|---|---|
| **[`00-设计思想.md`](00-设计思想.md)** | **角色宣言**：native 薄壳定义 / ADR-007 前后对比（1119→168 行）/ NativeEventSink broadcast no-op flush / NativeStorage 委托层 parity / 合法 ambient API 落点 | — |
| [`01-engine-loop-event-pump.mmd`](01-engine-loop-event-pump.mmd) | **事件泵全景**：transport/timer/command 三源 → Tick → tick_tx(cap=256) → biased select(reply 优先) → step → consume_effects → dispatch → Effect 兑现器 → PortReply 经 unbounded reply_tx 回灌闭环。🔴 死锁红线①②标注 | `engine_loop.rs` run_engine_loop_with_transports / 主循环 |
| [`02-bounded-spawner-backpressure.mmd`](02-bounded-spawner-backpressure.mmd) | **统一背压原语 BoundedSpawner 三态**：Persist(N=1 Block 保写序) / Http(N=8 Block 必达不丢) / HttpFire(N=8 DropNewest 可丢·满即丢当前条目)；有界队列 cap=1024 + 独立 unbounded reply_tx。🔴 红线①(reply_tx 永不有界) + 红线②(抽水绝不放泵循环)·单向 DAG 死锁安全证明 | `engine_loop.rs` struct BoundedSpawner / Overflow / submit / get_or_spawn_persist / spawn_http_pool |
| [`03-effect-dispatch-routing.mmd`](03-effect-dispatch-routing.mmd) | **Effect 兑现路由**：9 种 Effect 各自怎么被 native 兑现——Send→WS / Persist·PersistFire→spawner→host SQLite / Http·HttpFire→spawner→reqwest / Emit→broadcast event_sink / Schedule·CancelTimer→TimerRegistry / Request→deferred stub | `engine_loop.rs` dispatch_effects match + consume_effects |

## 五不变量（破坏须同步改图 + AGENTS.md）

刀2-driver：Persist / Http(必达) / HttpFire(可丢) 统一成 `BoundedSpawner`（有界 mpsc + N worker + 溢出策略 + 独立 unbounded reply_tx）。

1. **回灌必达**：PortReply 走独立 unbounded `reply_tx`，永不堵塞 worker。🔴 红线：reply_tx 绝不能改有界——否则 Block 死锁环回来。
2. **主循环不等 I/O（Block 传导回源）**：Persist / 必达 Http 入有界队列即返回；队满 `submit().await` 把背压传导回 tick_tx（慢而不丢）。HttpFire 走 DropNewest（满即丢当前条目 + warn）。🔴 红线：抽水绝不放泵循环。
3. **同 key 写顺序**：Persist per-table-set `BoundedSpawner` 强制 N=1（单消费者保序）；多表 ops 走组合键独立实例。Http/HttpFire 无写序要求（N=8）。
4. **回灌优先**：biased `select!` 把 `reply_rx` 排在 `tick_rx` 前，防 PortReply 饥饿。
5. **graceful drain**：`run_engine_loop` 返回 = 全部已接收 Persist + 必达 Http 已落地（shutdown：drop tx → join workers → drain replies）。HttpFire 可丢，不纳入保证。

## ⚠️ 已知缺口（图中标 ⚠️）

- **跨模块公平性（刀3）**：BoundedSpawner 并发名额全局共享、单队列 FIFO。单模块（IM）无饥饿；**第二个业务模块（presence/notify）接入前必修**——引入 per-module 配额 / by-module 轮转。🔴 绝不现在建优先级调度器（预建框架违 GOAL §2 YAGNI）。
- **HttpFire 池零生产者**：helix-im 三个 Http 的 PortReply 均 load-bearing，全为必达 `Http`，无一吐 `HttpFire`。本池为与 http_pool 类型对称而 eager 建；第二模块接入时重审/删。
- **Effect::Request 方向①**：HostRequester port 未接，当前 deferred stub（仅 tracing::warn）。core 侧闭环已由 `helix-core/tests/request_reply_replay.rs` 验证。
- **WS reconnect/backoff**：连接级重连退避在 `helix-driver-host` 的 `SharedWsClient` 侧，native 仅接生命周期 Tick（Connected/Disconnected）；退避策略尚残（见 transport.rs 头注）。

## 渲染

mermaid `.mmd` 可用 mermaid-cli / VS Code Mermaid 插件 / GitHub mermaid 渲染：

```bash
npx -y @mermaid-js/mermaid-cli -i 01-engine-loop-event-pump.mmd -o 01.svg
```
