# helix-driver-native 局部指令（就近优先，全局见根 AGENTS.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 契约（engine_loop 五不变量，破坏须同步改文档）

> 刀2-driver：Persist / Http(必达) / HttpFire(可丢) 统一成 `BoundedSpawner` 原语
> （有界 mpsc 队列 + N worker + 溢出策略 Block|DropNewest + 独立 unbounded reply_tx）。
> 三类只是参数差异，背压纪律集中一处（不再 per-type ad-hoc 散落）。

1. **回灌必达**：PortReply 走独立 unbounded reply_tx，永不堵塞 worker。
   🔴 红线：reply_tx 绝不能改有界——否则 Block 死锁环回来（泵 block 入队 → 不消费 reply → reply 满 → worker block 回报 → 不抽队列）。
2. **主循环不等 I/O（Block 传导回源）**：Persist / 必达 Http 入有界队列即返回；队满时 `submit().await` 把背压传导回 tick_tx（慢而不丢，绝不 drop）。HttpFire 走 DropNewest（try_send 满即丢当前条目 + warn，泵不阻塞）。
   🔴 红线：抽水绝不放泵循环——任何「dispatch 末尾 await worker 完成」都会引入自死锁。抽水在独立 worker task，submit 即返回循环。
3. **同 key 写顺序**：Persist per-table-set BoundedSpawner **强制 N=1**（单消费者保序）；多表 ops 走组合键独立实例（跨 key 不保证相对顺序——需有序就合进同一 Persist）。Http/HttpFire 无写序要求（N=8）。
4. **回灌优先**：biased select reply_rx 在前
5. **graceful drain**：「run_engine_loop 返回 = 全部已接收 Persist **已落地** + 必达 Http **已持久化(乐观态)**」。Persist drain 无上限（本地 DB 写，毫秒级，`shutdown` drop tx → join workers → drain replies）；**必达 Http drain 有上限**（`shutdown_with_timeout(HTTP_DRAIN_LIMIT=3s)`，在途 reqwest 超时则 abort，**生命周期绝不被网络 I/O 阻塞**，fix/lifecycle-net-decouple）。「必达」由乐观态(status=Sending)落库 + 重连重发对账承担，不由「drain 必等满网络」承担。HttpFire 可丢，不纳入「必达落地」保证。

### Block 模式死锁安全（与现有 Persist 模型同构，已压测验证）

泵阻塞在「入队 submit().await（满）」安全，当且仅当上面两条红线成立：① 抽水在独立 worker task ② reply_tx unbounded。依赖链是单向 DAG（泵等 worker，worker 不等泵）→ 无环 → 无死锁。BoundedSpawner 只是把已验证的 `get_or_spawn_worker` 形态抽成通用原语复用到 Http，不引入新死锁面。

### ⚠️ 已知缺口：跨模块公平性（刀3，第二个业务模块接入前必修）

当前 BoundedSpawner 的并发名额（N 个 worker / 单有界队列）**全局共享、单队列 FIFO**。单模块（IM）下 FIFO 无饥饿问题。但**多模块时无饥饿保证**：第二个业务模块（presence / notify）接入前，必须引入 **per-module 名额配额 或 by-module 轮转**（参照 Dioxus `dirty_scopes` 按 height 确定序的调度公平性），否则一个模块的洪峰会饿死另一个模块的 effect。

🔴 **绝不现在建优先级调度器**（预建框架违 GOAL §2 YAGNI）。当前单模块无此问题，缺口仅在第二模块接入这一明确触发点修复。

## Storage 约定

- 三池：sync 写(1) / 用户写(1) / 读(4)，共享 WAL；batch_upsert 整批一事务（E2，写放大 ~1.0x）
- 本 crate 是 SystemTime/线程等 ambient API 的**唯一合法落点**（clippy 在此 allow）
- rows→bytes / HTTP envelope JSON 约定真源在 `helix-driver-host`（`codec` / `http_envelope`）；本 crate engine_loop 仅 re-export 转出，helix-im 反序列化端必须对齐

## 改动验收

```bash
cargo test -p helix-driver-native
cargo test --release -p helix-driver-native --test stress_engine --test stress_storage -- --test-threads=1   # 改泵/存储时必跑
```
