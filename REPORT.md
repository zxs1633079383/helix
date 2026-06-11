# REPORT — E1/E2 driver-native 修复（fix/e1-e2-driver-native）

> branch: `fix/e1-e2-driver-native`
> baseline HEAD: `0ee0de2`（test(承压): 并发压测三件套入库）
> worktree cwd: `worktrees/fix-e12-driver/`

## Guard 自检

```
pwd  : /Users/mac28/workspace/rustWorkspace/helix/worktrees/fix-e12-driver   ✅
HEAD : fix/e1-e2-driver-native                                                ✅
```

## 改动文件

| 文件 | 改动 | 说明 |
|---|---|---|
| `crates/helix-driver-native/src/storage.rs` | +33/-23 | E2：batch_upsert 包显式事务 |
| `crates/helix-driver-native/src/engine_loop.rs` | +～130/-～80 | E1：泵死锁修复 |
| `crates/helix-driver-native/tests/stress_engine.rs` | +111/-53 | E1 正向断言 + windowed 对照组 |

黑名单未触碰：helix-core/**、Cargo.toml/Cargo.lock、根目录三文件、docs/**。

## 修复前后数字对照

### E2 — batch_upsert 写吞吐（5000 行单批，--release，SQLite WAL）

| 指标 | 修复前（逐行 autocommit）| 修复后（整批一事务）| 倍率 |
|---|---|---|---|
| 5000 行单批写入延迟 | 369 ms | 40–60 ms | 6–9x 加速 |
| 行吞吐 | 13,537 rows/s | 82,763–122,786 rows/s | 6–9x |
| 写放大系数（vs 显式事务基线）| 9.6x | ~1.0x（齐平）| 消除 |

### E2 — 39 行 × 100 批

| 指标 | 修复前 | 修复后 |
|---|---|---|
| 3900 行总时间 | ~300 ms（评估报告）| 48 ms |
| 行吞吐 | ~13K rows/s | 81,110 rows/s |

### E1 — 端到端泵吞吐（10k persist-roundtrip，--release，4 worker threads）

| 场景 | 修复前 | 修复后 |
|---|---|---|
| 裸灌 >256 Command（无限流）| 永久死锁 | 全部完成，954ms，10,472 rps |
| window=100 背压对照 | 9.7K rps（评估报告）| 10,573 rps（稳定）|

## 设计决策

### E2：整批一事务（而非多值 INSERT 或 pipeline）

SQLite 写瓶颈在 WAL fsync（每次 COMMIT 触发一次）。逐行 autocommit = N 次 fsync；
整批一事务 = 1 次 fsync。多值 INSERT 需动态拼绑定参数个数，与 sqlx prepared statement
缓存不兼容且代码更复杂；整批事务最简单、最安全、收益等价。

回滚语义：sqlx Transaction drop-on-error 自动 ROLLBACK，调用方 cursor 不推进，
下次重发整批，幂等 upsert 安全。

### E1：四不变量设计决策

**回灌走独立 unbounded channel（不扩大 tick_rx 容量）**

扩大容量治标不治本：洪峰大小未知，任何有界容量都有临界点。unbounded 语义正确：
in-flight PortReply 数量受 corr_map 登记数自然有界，不会无限增长。

**per-table 长驻 worker（而非每批 spawn）**

每个 table 一个长驻 tokio task + 自己的 unbounded queue，单消费者天然保序，
无需额外同步。table 种类数有限（生产中不超过几十个），worker 开销可忽略；
各 table worker 之间天然并发（_helix_monotonic 与 messages 可同时写）。

**biased select!（reply_rx 优先）**

不带 biased 时 select! 在两路都有数据时随机选，reply 可能被 Command 长时间压后。
biased 保证只要回灌队列非空就优先消费，防饥饿。

**同组写顺序保证机制**

per-table worker 是单消费者，unbounded_channel 保证入队顺序 = 出队顺序。
dispatch_effects 是主循环内的同步调用，入队顺序 = step 产生 Effect 的顺序
= 跨 tick 的时序顺序，严格保序。

## 全量回归

```
cargo test --release -p helix-driver-native -- --nocapture --test-threads=1

unit tests     : 27 passed
integration    :  3 passed
stress_engine  :  2 passed (no_window + windowed)
stress_storage :  3 passed
doc-tests      :  2 ignored

TOTAL: 35 passed / 0 failed
```
