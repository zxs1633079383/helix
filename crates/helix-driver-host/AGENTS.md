# helix-driver-host 局部指令（就近优先，全局见根 AGENTS.md）

> 📐 **设计图（当前能力 + 流程）→ [`docs/design/`](docs/design/)**（mermaid 流程图，先读其 `README.md` 索引）。

## 本 crate 是什么

PC Tauri（native）与 Flutter FFI **共享**的 host driver 层（ADR-007）：

- **存储层**：SQLite schema/migration、rusqlite `Storage` 实现、`StorageOp → PortOutcome` 兑现、`rows_to_reply_bytes`/`rows_from_reply_bytes` 编解码。
- **pump 核（事件泵原语，本轮下沉）**：`engine`（泛型事件泵壳 `run_engine_loop`，over Storage/HttpRequester/EventSink/FrameSender/Clock）+ `dispatch`（Effect 分流）+ `spawner`（`BoundedSpawner` 有界并发原语）+ `pools`（Persist/Http 池构造）+ `owned_effect`（`&[Effect]` 借用脱离）+ `http_envelope`（HTTP 响应 → PortReply JSON 信封）+ `timer`（`TimerRegistry`）。连接/收帧/关闭/重连不在 core port，由 host/native/FFI 具体生命周期管理。

`helix-driver-native` 与 `helix-driver-ffi` 只保留平台壳：装配具体 ports + tick 源接线 + EventSink egress 特化（native=broadcast / ffi=批处理 vtable），共用此泛型 pump 核，不重复实现存储/泵。

- **`BatchSink` egress 批处理钩子（`engine.rs`）**：`trait BatchSink: EventSink { fn flush(&self) {} }`（默认 no-op）。`run_engine_loop` 的 `E` bound = `BatchSink`，在**每次 `dispatch_effects`（本 step effects）之后**调一次 `flush()`。native `impl BatchSink for NativeEventSink {}`（broadcast 即时发，no-op）；ffi `FfiEventSink::flush` 把本 step 攒的整批 DomainEvent 编码成单 buffer，一次过 vtable 回调（HX-C007，绝不逐条过 FFI 边界）。flush 钩子在 host 不在 core——core 的 `EventSink` 不认识「批边界」运行时概念（HX-C001 边界）。
- **`RecordingSink<S>` 外部宿主投影录放装饰器范例（`instrument.rs`，add-only）**：包任意 `S: BatchSink`，每条 `emit` 旁路给注入回调（录放/日志/对账）再透传，`flush` 原样透传（HX-C007 批边界不变）。**关键**：投影面装饰器必须同时 `impl EventSink + BatchSink`（引擎 `E` bound 是 `BatchSink` 非 `EventSink`，只实现 `EventSink` 装不进 `EngineDeps.event_sink`）。供 loopforge 等外部组装根抄形态抓 facet②。完整 recipe → [`docs/external-host-instrumentation-recipe.md`](docs/external-host-instrumentation-recipe.md)。

## 本 crate 不变量（破坏 = 双端存储/泵语义漂移）

1. **单一真源**：messages SQL、monotonic 表、StorageOp 语义、rows 编解码、**pump 核原语 + 五不变量** = 此 crate 唯一定义点；native/ffi 复用，禁各写一份（否则 PC 与手机端语义分叉）。
2. **rows↔bytes JSON 约定**：与 `codec.rs` / `http_envelope.rs` 头注对齐——Text→string / Integer→number / Real→number / Null→null；HTTP 信封 `{status,headers:[[k,v]...],body:base64}`；helix-im 反序列化端按此还原。
3. **热路径 O(1)**：batch_upsert 整批一事务（写放大 ~1.0x）；单 INSERT / UPDATE WHERE PK；禁 read-modify-write（HX-C005）。
4. **migration 不可逆**：只增不改历史 migration；schema 演进加新 migration，不回改旧的。
5. **driver 层 ambient 合法**：rusqlite 同步 C 调用、SystemTime 等在此/上层 allow；FFI 侧同步调用须 `spawn_blocking` 包裹（HX-C006/ADR-005，不阻塞 tokio worker）。

## 事件泵五不变量（真源在 `engine.rs` 头注，破坏须同步改本节 + native AGENTS.md）

1. **回灌必达**：PortReply 走独立 unbounded reply_tx（🔴 绝不改有界，否则 Block 死锁环回来）。
2. **主循环不等 I/O**：Persist / 必达 Http 入有界队列即返回；满则 Block 传导回 tick_tx（🔴 抽水绝不放泵循环）。HttpFire DropNewest（满即丢当前条目，泵不阻塞）。
3. **同 key 写顺序**：Persist per-table-set BoundedSpawner 强制 N=1（单消费者保序）。
4. **回灌优先但不饿死入站**：biased select reply_rx 在前；连续 64 条 PortReply 后若 tick 已排队，强制消费 1 条入站，再恢复回灌优先。
5. **graceful drain**：`run_engine_loop` 返回 = 全部已接收 Persist **已落地** + 必达 Http **已持久化(乐观态)**。必达 Http drain **有上限**（`HTTP_DRAIN_LIMIT`，3s）：在途 reqwest 超时则 abort，**生命周期绝不被网络 I/O 阻塞**（fix/lifecycle-net-decouple）。「必达」由乐观态(status=Sending)落库 + 重连重发对账承担，不由「drain 必等满网络」承担。Persist drain 无上限（本地 DB 写，毫秒级，保「同 key 写序+全落地」）。HttpFire 可丢，不纳入。

> Block 模式死锁安全：抽水在独立 worker task + reply_tx unbounded → 依赖链单向 DAG（泵等 worker，worker 不等泵）→ 无环。已压测验证（native stress_engine 复用此核）。

## 改动验收

```bash
cargo test -p helix-driver-host
# 改 schema/StorageOp/rows codec / pump 核后，native + ffi 两端都要回归（共享语义）
cargo check -p helix-driver-native -p helix-driver-ffi
# 改 pump 核（engine/dispatch/spawner/pools/timer）后，native 泵回归是零行为改变安全网：
cargo test -p helix-driver-native --test stress_engine --test stress_storage
```
