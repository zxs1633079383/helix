# helix-driver-host · 设计图索引

> `helix-driver-host` = PC Tauri（native）与 Flutter FFI **共享**的 host I/O 层（ADR-007 单一真源）。
> native / ffi 只留薄壳，存储 / 网络复杂度全沉淀在此。本目录 mermaid 图从真实 src 提炼，
> 真源文件：`src/{storage.rs, network.rs, ops.rs, codec.rs}`。

| 文档 / 图 | 一句话 | 真源 | 关联约束 |
|---|---|---|---|
| **[`00-设计思想.md`](00-设计思想.md)** | **架构意图与核心决策备忘**：北极星单一真源 / parity by construction / C++ 直连转折 / BatchSink flush hook / 读写三池 / 工程化教训 | — | ADR-007 · §§1-8 |
| [`01-storage-read-write-split-pool.mmd`](01-storage-read-write-split-pool.mmd) | rusqlite 读写分离池：写连接 **N=1**（串行保序，WAL 单写者）/ 只读连接 **N=4**（READ_ONLY，WAL 并发快照），Semaphore 限并发 + spawn_blocking 隔离 + WAL/PRAGMA 在 open 设 | `storage.rs` | HX-C006 · ADR-005/007 |
| [`02-storageop-to-sql.mmd`](02-storageop-to-sql.mmd) | 7 种 StorageOp（BatchUpsert/MonotonicUpsert/BatchUpdate/GuardedBump/Get/Scan/BatchDelete）各自的 SQL 形态 + O(1) 热路径纪律（单 INSERT/UPDATE/DELETE WHERE PK，禁 read-modify-write，固定模板 `&'static` SQL + prepare_cached） | `ops.rs` · `storage.rs` | HX-C005 · ADR-007 |
| [`03-http-header-injection.mmd`](03-http-header-injection.mmd) | HostNetworkConfig（启动期配置）+ HostHeaderRegistry（parking_lot RwLock 运行时 header 快照，无 poison）→ 每个 HTTP 请求 / WS 握手注入最新快照 → reqwest 出站（HTTP 与 WS 共享同一 registry/token） | `network.rs` | HTTP-4 · ADR-007 |
| [`04-async-shell-wraps-sync-io.mmd`](04-async-shell-wraps-sync-io.mmd) | 异步外壳包同步 I/O：`#[async_trait]` async fn → `spawn_blocking`（open / execute_raw / with_reader / 4 个写 op）→ 同步 rusqlite C 调用。**异步是调度手段，真 I/O 同步**，并发/阻塞隔离全在 driver 层（core 不持 runtime） | `storage.rs` | HX-C006 · ADR-005 |

| [`05-storage-rw-concurrency.mmd`](05-storage-rw-concurrency.mmd) | 读写/读读/写读三类并发交织图：三入口（WS 写 / HTTP 读写 / query 读）→ HostStorage，写路径 BoundedSpawner N=1 写序 + ReadPool Semaphore(4) 背压 + WAL 快照并发，含 FFI current_thread vs native 差异 | `storage.rs` · `engine.rs` · `spawner.rs` | HX-C005 · HX-C006 · 五不变量 |
| [`06-sync-v2-write-load.mmd`](06-sync-v2-write-load.mmd) | sync-v2 三路径写负载流图：path1 全量 upsert / path2 PATCH / path3 未读自增 + cursor 推进 + channel_event_cursor，经 BoundedSpawner N=1 → HostStorage WAL，含 PortReply 回灌闭环 | `helix-im/{channel,module,acl}` · `engine.rs` | HX-C005 · HX-C008 · 五不变量 |

**设计规范文档（并发正确性 + 压测）**

| 文档 | 一句话 | 真源 |
|---|---|---|
| [`07-storage-rw-concurrency-spec.md`](07-storage-rw-concurrency-spec.md) | 读读/读写/写读三类并发正确性不变量 + 死锁饥饿面分析 + FFI vs native 差异 + 现有压测覆盖缺口矩阵 + 6 个待补场景设计 + 下一步 workflow scope | `storage.rs` · `spawner.rs` · `engine.rs` · `tests/` |

## 不变量速记（破坏 = 双端存储漂移）

- **单一真源**：messages SQL / monotonic 表 / StorageOp 语义 / rows 编解码 = 此 crate 唯一定义点，native/ffi 复用不重写。
- **读写分离**：写 op → 1 写连接串行（WAL 单写者本就正确）；读 op（get/scan）→ 4 只读连接 WAL 并发快照。
- **热路径 O(1)**（HX-C005）：batch_upsert 整批一事务（写放大 ~1x）；monotonic cursor 用 `&'static` SQL + MAX guard 防回退。
- **spawn_blocking 隔离**（HX-C006/ADR-005）：所有同步 rusqlite C 调用包 `spawn_blocking`，不阻塞 tokio worker。
- **header 注入**（HTTP-4）：parking_lot RwLock 无 poison，HTTP/WS 共享 registry，client 只注入快照，鉴权在 Helix 之外。

## rows↔bytes codec（`codec.rs`，非图但同属本层契约）

`rows_to_reply_bytes` / `rows_from_reply_bytes` JSON 约定（与 `helix-driver-native/engine_loop.rs` 头注对齐）：
Text→string / Integer→number / Real→number / Null→null / Blob→JSON 字节数组（M2，二进制无损）。
