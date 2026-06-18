# HostStorage 读写并发正确性规范

> 真源：`src/storage.rs`（HostStorage / ReadPool）· `src/spawner.rs`（BoundedSpawner N=1）· `src/engine.rs`（run_engine_loop）  
> 关联流程图：[`05-storage-rw-concurrency.mmd`](05-storage-rw-concurrency.mmd) · [`06-sync-v2-write-load.mmd`](06-sync-v2-write-load.mmd)  
> 关联约束：HX-C005 · HX-C006 · HX-C008 · 五不变量（engine.rs 头注）

---

## 1. 并发模型概述

```
HostStorage
├── writer: Arc<Mutex<Connection>>  (READ_WRITE | CREATE | NO_MUTEX)
│   └── 所有写 op：batch_upsert / batch_update / monotonic_upsert
│       经 BoundedSpawner N=1 → spawn_blocking → writer.lock()
│
└── readers: Option<Arc<ReadPool>>  (文件库启用，内存库不共享则 None)
    ├── permits: Arc<Semaphore(4)>   — 限并发 = 连接数
    ├── idle: Mutex<Vec<Connection>> (READ_ONLY | NO_MUTEX)
    └── get / scan 经 spawn_blocking → acquire → pop → 用完 push 归还

WAL 模式：journal_mode=WAL + synchronous=NORMAL + busy_timeout=5000
NO_MUTEX：连接已被 Rust Mutex/只读池独占，消除 SQLite 内部串行锁双重开销（B2）
```

所有 rusqlite 同步 C 调用全包 `spawn_blocking`（HX-C006），不阻塞 tokio worker 线程。

---

## 2. 三类并发语义正确性不变量

### 2.1 读读（Read-Read）

**正确性结论：✅ 并发正确，池满有背压，内存库退化串行但语义正确。**

| 条件 | 行为 | 正确性 |
|---|---|---|
| N 个读者同时请求，N ≤ 4 | 各取一个 READ_ONLY 连接，WAL 快照独立，真并发 | ✅ |
| N 个读者同时请求，N > 4 | 第 N-4 个及以上 `acquire_owned().await` 异步挂起（不占线程），等前面归还 permit 后继续 | ✅（背压，无死锁）|
| `target_supports_shared_readers=false`（独占内存库） | `readers=None`，读退到 `writer.lock()` 串行 | ✅ 正确，并发度为 1 |

**风险：** 独占内存库（`:memory:` 或 `mode=memory` 无 `cache=shared`）下读退化为串行，并发读压力大时吞吐会显著下降。生产路径使用文件 WAL 库，此风险不影响生产。

**代码证据：** `storage.rs:128-169`（`with_reader` 实现）、`storage.rs:363-371`（`target_supports_shared_readers`）。

---

### 2.2 读写（Read-Write）

**正确性结论：✅ WAL 快照隔离，读写真并发，cursor 单调性条件成立。**

- **WAL 快照隔离**：读者（READ_ONLY 连接）读取 writer 上次已提交的快照，不被 writer 的进行中事务阻塞，也不会看到 writer 未提交的脏数据（`storage.rs:16` 头注："WAL 模式下读者读快照、不阻塞写者，写者也不阻塞读者 → 真并发"）。
- **cursor 单调性成立的条件**：`monotonic_upsert` 走写连接（单写者串行）+ `ON CONFLICT ... WHERE excluded.value > value`（MAX guard）。并发 reader 看到的是上次已提交的 cursor 值，不存在两个 writer 争抢同一 cursor 的情况（N=1 BoundedSpawner 保写序）。
- **边界快照**：读者若在 writer 提交前发起 read，读到旧快照；提交后发起的读取新快照。这是预期的 MVCC 行为，对 sync-v2 的读路径（Get channel / Scan messages）是正确的：sync pull 拿到的 channel 数据是上次提交后的一致快照。

**风险：** 若业务逻辑要求「写完立刻读新值」（read-your-writes），需在同一 writer 连接上读。当前 HostStorage 不提供此语义——读永远走 ReadPool，写走 writer，不保证 read-your-writes。此语义缺口在 helix-im 的 sync 流程中**目前不是问题**（读的是 driver 的 scan 结果，由 PortReply 经 engine 异步回灌，不需要同步 read-your-writes）。

**代码证据：** `storage.rs:280-296`（`monotonic_upsert` MAX guard）、`storage.rs:120-170`（`with_reader` 走 ReadPool）。

---

### 2.3 写读（Write-Read）

**正确性结论：✅ writer Mutex 不阻塞 WAL reader，读写真并发，BoundedSpawner N=1 保写序。**

- `writer.lock()` 是 Rust `std::sync::Mutex`，仅在持有 writer 连接期间阻塞**其他写者**（但 writer N=1，无其他并发写者，实际无争抢）。
- WAL reader（READ_ONLY 连接）**不持有** writer mutex，WAL 文件级并发读不被 writer 的 mutex 阻塞。
- **强证明：** `storage_conformance.rs:417-525`（`reads_proceed_while_write_holds_connection`）用递归 CTE 长时占用写连接，同时并发 8 个 scan，断言读的总墙钟 < 写的占用时长，证明读在写持锁期间已全部返回。

**风险：** `batch_upsert` 用 `conn.transaction()` 打开事务（整批一事务，E2 不变量），事务期间写连接被 Mutex 保护，期间其他调用 `batch_upsert` 的路径会在 `writer.lock()` 阻塞，直到事务提交。由于写路径经 BoundedSpawner N=1 串行化，此阻塞在设计内，无问题。

---

## 3. FFI current_thread vs native multi_thread 差异

| 维度 | native（multi_thread） | FFI（current_thread）|
|---|---|---|
| tokio runtime 形态 | `new_multi_thread()`（默认 CPU 核数 worker）| `new_current_thread().enable_all()`（单 worker 线程）|
| `spawn_blocking` 行为 | 卸到独立 blocking pool（默认 512 线程）| **同样**卸到独立 blocking pool（tokio `current_thread` 支持 `spawn_blocking`，必须 `enable_all()` 否则 park 异常）|
| storage 并发度 | blocking pool 可多路并发执行 rusqlite 调用 | blocking pool 同样可多路并发（blocking pool 不受 current_thread 单 worker 限制）|
| core/engine 并发 | 泵在多 worker 之一跑（但 engine loop 本身是单 select 主循环）| 泵在专属单线程跑（GOAL §1.3 core 单线程）|
| 差异总结 | | **无功能差异**：storage I/O 并发度相同（都靠 blocking pool），差别仅在 pump 线程模型（single vs multi worker thread）|

**关键一句话：** `current_thread` + `enable_all()` 下 `spawn_blocking` 正常卸到 blocking pool，不阻塞 pump 单线程，storage 并发行为与 native 等同。（`pump.rs:65-68` 代码注释："漏 enable_all → spawn_blocking await / timer park 异常"）

---

## 4. 死锁/饥饿面分析

### 4.1 泵线程 + spawn_blocking + reply_tx 回灌链路

```
pump loop (biased select)
  → Tick::Inbound → step → Effect::Persist
  → BoundedSpawner.submit().await （满则 Block，背压回 tick_tx）
  → worker spawn_blocking → writer.lock() → rusqlite I/O
  → PortOutcome → reply_tx.send（unbounded，不会阻塞）
  → Tick::PortReply → pump biased select 优先消费
```

**无死锁**：依赖链单向 DAG（泵等 worker，worker 不等泵）。五不变量保证：① reply_tx unbounded（不变量1）② worker 独立 task 不在泵循环内阻塞等待（不变量2）③ biased select 优先消费 reply_rx（不变量4）。已压测验证（`stress_engine.rs` E1 修复验证）。

**Block 模式背压链**：BoundedSpawner 满 → `submit().await` 阻塞泵 → `tick_tx`（cap=256）背压到上游（WS 入站/HTTP sync pull）→ 自然限流，不死锁。

### 4.2 读路径 BoundedSpawner 覆盖缺口

**⚠️ 覆盖缺口**：BoundedSpawner N=1 是**写路径**（Effect::Persist）的串行化原语。读路径（Get/Scan）**不经 BoundedSpawner**，直接走 `with_reader()` → ReadPool Semaphore。

- 这意味着：并发读**不受 BoundedSpawner 的五不变量约束**（特别是「队满 Block 传导背压」）。
- 读的背压机制是 Semaphore(4)：超过 4 个并发读时 `acquire().await` 挂起，靠异步任务调度器自然排队。
- **当前无死锁风险**：读不回写，读路径不存在回灌循环。
- **潜在饥饿风险**：若大量并发读（>4）积压在 Semaphore 上，且读路径的 spawn_blocking 任务耗时较长（慢查询），可能导致读饥饿（但不影响写路径）。当前 helix-im 的读量相对写量小，此风险低。

### 4.3 helix_query（FFI 读路径）

当前 `helix_query` 是 stub（`FFI_UNIMPLEMENTED`），不走 ReadPool，无并发问题。一旦实装，需注意：FFI 同步调用须经 `spawn_blocking`（HX-C006 MAJ-4），否则阻塞 C++ 调用线程。

---

## 5. 压测设计

### 5.1 现有覆盖 vs 缺口矩阵

| 场景 | 现有文件 | 覆盖状态 | 说明 |
|---|---|---|---|
| 单写路径吞吐（batch_upsert）| `stress_storage.rs:72` | ✅ | 39×100 / 5000 单批 |
| 单写并发（100 batch 同时并发写）| `stress_storage.rs:166` | ✅ | 100 并发 + 32 并发读 |
| 写读真并发（写持锁期间读可完成）| `storage_conformance.rs:417` | ✅ | 强证明：读在写 CTE 期间返回 |
| cursor 热路径 monotonic_upsert × 1000 | `stress_storage.rs:238` | ✅ | |
| BoundedSpawner 写序保证（N=1 保序）| `spawner.rs:228` | ✅ | 单元测试 test_n1_preserves_order |
| BoundedSpawner Block 不死锁 + 全落地 | `spawner.rs:192` + `stress_engine.rs` | ✅ | E1 死锁修复验证 |
| engine graceful drain 全落盘 | `stress_engine.rs:302` | ✅ | shutdown drain 验证 |
| **读读并发（N≤4 真并发 / N>4 Semaphore 阻塞）** | — | **✗** | 无专项并发 N>4 读测试 |
| **读写并发 WAL 快照隔离（reader 看旧快照不看脏数据）** | `storage_conformance.rs:417` 部分覆盖 | ⚠️ 部分 | 现有测试证明读写不互斥，但无"快照隔离"断言（读者是否看到旧值）|
| **sync-v2 path1 全量 upsert（channel 全量同步负载）** | — | **✗** | 无 channel batch_upsert 专项 |
| **sync-v2 path2 PATCH（batch_update 在并发写下）** | — | **✗** | 无 batch_update 并发测试 |
| **sync-v2 path3 未读自增（SQL 形态待确认）** | — | **✗** | 需确认实现是否 O(1)，有 HX-C005 风险 |
| **sync-v2 cursor 并发单调性（多 channel 并发 cursor push）** | — | **✗** | monotonic_upsert 压测是单 channel 顺序写 |
| **三类入口混合 flood（WS 写 + HTTP 读写 + 并发读同时）** | 部分（stress_storage 100并发写+32读）| ⚠️ 部分 | 未覆盖真实 engine dispatch 层混合场景 |
| **FFI current_thread vs native multi_thread 对照** | — | **✗** | 无两种 runtime 对照压测 |
| **host 共享核（native + ffi 公用 HostStorage 实例）** | — | **✗** | 无跨驱动共享实例压测 |
| **BoundedSpawner 读路径饥饿（Semaphore 满时）** | — | **✗** | 无读路径背压测试 |

### 5.2 待补压测场景设计

#### 场景 A：读读并发 N>4 Semaphore 背压

**目标文件**：扩展 `crates/helix-driver-host/tests/storage_conformance.rs`（新增 test）

**逻辑**：
1. 打开文件 WAL 库（`file_storage()`）
2. 预填 100 行
3. 同时 spawn 16 个 `scan()` 任务（N=16 > READ_POOL_SIZE=4）
4. 计时：所有 16 个读完成的总墙钟

**判据**：
- ✅ 16 个读全部成功（无 PortError）
- ✅ 总墙钟 < 16 × base（并发借用，非全串行）
- ✅ 总墙钟 > 4 × base（分批通过 Semaphore，不是瞬间并发）
- ✅ 结果正确（每个读拿到相同 100 行）

#### 场景 B：WAL 快照隔离语义断言

**目标文件**：扩展 `crates/helix-driver-host/tests/storage_conformance.rs`

**逻辑**：
1. 打开文件 WAL 库，预填 id=1 行 `value=100`
2. 起一个"慢写"任务：`BEGIN IMMEDIATE` → 更新 id=1 `value=200` → 等 200ms → `COMMIT`
3. 在 COMMIT 之前发起 `get(id=1)` 读
4. 在 COMMIT 之后发起 `get(id=1)` 读

**判据**：
- ✅ COMMIT 前的读：`value=100`（旧快照，不脏读）
- ✅ COMMIT 后的读：`value=200`（新快照）

#### 场景 C：sync-v2 多 channel 并发 cursor 单调性

**目标文件**：扩展 `crates/helix-driver-native/tests/stress_storage.rs`（新增函数）

**逻辑**：
1. 打开文件 WAL 库
2. 100 个 channel，每 channel 独立 goroutine 顺序推进 100 次 monotonic_upsert（scope_key=`channel:{id}`）
3. 所有任务完成后，对每个 channel 执行 `get(_helix_monotonic, scope_key=channel:{id})`

**判据**：
- ✅ 每个 channel 的 value == 100（MAX guard 确保不回退）
- ✅ 无死锁（100×100=10000 次写在时限内完成）
- ✅ 无 cursor 回退（验证 N=1 写序 + MAX guard 联合保证）

#### 场景 D：sync-v2 path3 未读自增形态验证

**目标文件**：新增 `crates/helix-im/tests/increment_concurrency_test.rs`（或扩展现有 `increment_flow_test.rs`）

**前提**：先通过代码 review 确认 path3（未读自增）的 SQL 形态（是 `UPDATE SET unread = unread + 1`（read-modify-write，违 HX-C005）还是 `monotonic_upsert` 形态）。

**逻辑（若 path3 走 batch_update SQL INCREMENT）**：
1. 注册 1 个 channel
2. 并发喂 10 个 `increment_channel` 类型的 `Tick::Inbound`
3. 期待每条产出 1 个 `Effect::Persist(BatchUpdate)` / `Effect::Persist(MonotonicUpsert)`
4. 驱动兑现后读 unread_count

**判据**：
- ✅ unread_count == 10（所有增量都落地）
- ❌ 若 path3 走 read-modify-write（先读后写）→ HX-C005 违反，需要重新设计为 append-only log 或 SQL 原子自增

#### 场景 E：三类入口 engine 层混合 flood

**目标文件**：扩展 `crates/helix-driver-native/tests/stress_engine.rs`（新增函数）

**逻辑**：
1. 起 engine（native）+ 文件 WAL 库
2. 并发向 tick_tx 发送：500 个写命令（Persist MonotonicUpsert）+ 100 个读命令（Persist Get）
3. 等所有 PortReply 回灌

**判据**：
- ✅ 无死锁（120s 内全部完成）
- ✅ 全部写 PortReply 到达（500 条）
- ✅ 全部读 PortReply 到达（100 条）
- ✅ cursor 最终值 == 500（单调）

#### 场景 F：FFI current_thread vs native 对照基准

**目标文件**：新增 `crates/helix-driver-ffi/tests/pump_stress_test.rs`（若 FFI 测试框架支持）

**逻辑**：
- native：用 `stress_engine.rs` 的 10k pump 测试，记录 throughput
- FFI：用 FFI pump 起相同负载（10k helix_command），记录 throughput

**判据**：
- ✅ FFI throughput ≥ native × 0.8（10+% 差异在线程调度开销内可接受）
- ✅ 无死锁，无 PortReply 丢失
- ✅ 确认 current_thread + enable_all 的 spawn_blocking 正常运作

---

## 6. 建议的下一步 workflow scope

### 优先级 P0（阻塞 sync-v2 正确性验证）

1. **确认 path3（未读自增）SQL 形态**：读 `helix-im/src/` 确认 `increment_channel` handler 的写 SQL，验证是否存在 read-modify-write 违反 HX-C005。若违反，需在 workflow 内修复（改为原子 SQL 自增或独立 append-only 表）。

2. **WAL 快照隔离断言（场景 B）**：扩展 `storage_conformance.rs`，明确验证脏读不存在。

### 优先级 P1（并发正确性补全）

3. **场景 A（读读并发 N>4）**：扩展 `storage_conformance.rs`，验证 Semaphore 背压无死锁。

4. **场景 C（多 channel cursor 并发单调性）**：扩展 `stress_storage.rs`，100 channel 并发 cursor push。

5. **场景 E（engine 层混合 flood）**：扩展 `stress_engine.rs`，写+读命令并发。

### 优先级 P2（对照 + FFI 验证）

6. **场景 D（increment 并发）**：在 path3 形态确认后实现。

7. **场景 F（FFI vs native 对照）**：与 FFI driver 实装（helix_query stub 填充）一起做。

### 复用现有 harness

- 所有场景优先扩展现有文件，不新建 crate：
  - `storage_conformance.rs`：A、B 场景
  - `stress_storage.rs`：C 场景
  - `stress_engine.rs`：E 场景
- 判据格式遵循现有模式：`assert!` + 墙钟上限 + 结果行数校验

### 对抗验证检查点

workflow 完成后，`architecture-drift-review` 应覆盖：
- path3 写形态是否满足 HX-C005（O(1) 无 RMW）
- 新增压测的 `target_supports_shared_readers` 路径是否覆盖文件库（非内存库）
- cursor 并发场景是否验证了 `WHERE excluded.value > value`（MAX guard）而非仅验证了写入成功
