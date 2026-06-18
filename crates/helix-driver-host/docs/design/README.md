# helix-driver-host · 设计图索引

> `helix-driver-host` = PC Tauri（native）与 Flutter FFI **共享**的 host I/O 层（ADR-007 单一真源）。
> native / ffi 只留薄壳，存储 / 网络复杂度全沉淀在此。本目录 mermaid 图从真实 src 提炼，
> 真源文件：`src/{storage.rs, network.rs, ops.rs, codec.rs}`。

| 图 | 一句话 | 真源 | 关联约束 |
|---|---|---|---|
| [`01-storage-read-write-split-pool.mmd`](01-storage-read-write-split-pool.mmd) | rusqlite 读写分离池：写连接 **N=1**（串行保序，WAL 单写者）/ 只读连接 **N=4**（READ_ONLY，WAL 并发快照），Semaphore 限并发 + spawn_blocking 隔离 + WAL/PRAGMA 在 open 设 | `storage.rs` | HX-C006 · ADR-005/007 |
| [`02-storageop-to-sql.mmd`](02-storageop-to-sql.mmd) | 5 种 StorageOp（BatchUpsert/MonotonicUpsert/BatchUpdate/Get/Scan）各自的 SQL 形态 + O(1) 热路径纪律（单 INSERT/UPDATE WHERE PK，禁 read-modify-write，固定模板 `&'static` SQL + prepare_cached） | `ops.rs` · `storage.rs` | HX-C005 · ADR-007 |
| [`03-http-header-injection.mmd`](03-http-header-injection.mmd) | HostNetworkConfig（启动期配置）+ HostHeaderRegistry（parking_lot RwLock 运行时 header 快照，无 poison）→ 每个 HTTP 请求 / WS 握手注入最新快照 → reqwest 出站（HTTP 与 WS 共享同一 registry/token） | `network.rs` | HTTP-4 · ADR-007 |
| [`04-async-shell-wraps-sync-io.mmd`](04-async-shell-wraps-sync-io.mmd) | 异步外壳包同步 I/O：`#[async_trait]` async fn → `spawn_blocking`（×7）→ 同步 rusqlite C 调用。**异步是调度手段，真 I/O 同步**，并发/阻塞隔离全在 driver 层（core 不持 runtime） | `storage.rs` | HX-C006 · ADR-005 |

## 不变量速记（破坏 = 双端存储漂移）

- **单一真源**：messages SQL / monotonic 表 / StorageOp 语义 / rows 编解码 = 此 crate 唯一定义点，native/ffi 复用不重写。
- **读写分离**：写 op → 1 写连接串行（WAL 单写者本就正确）；读 op（get/scan）→ 4 只读连接 WAL 并发快照。
- **热路径 O(1)**（HX-C005）：batch_upsert 整批一事务（写放大 ~1x）；monotonic cursor 用 `&'static` SQL + MAX guard 防回退。
- **spawn_blocking 隔离**（HX-C006/ADR-005）：所有同步 rusqlite C 调用包 `spawn_blocking`，不阻塞 tokio worker。
- **header 注入**（HTTP-4）：parking_lot RwLock 无 poison，HTTP/WS 共享 registry，client 只注入快照，鉴权在 Helix 之外。

## rows↔bytes codec（`codec.rs`，非图但同属本层契约）

`rows_to_reply_bytes` / `rows_from_reply_bytes` JSON 约定（与 `helix-driver-native/engine_loop.rs` 头注对齐）：
Text→string / Integer→number / Real→number / Null→null / Blob→JSON 字节数组（M2，二进制无损）。
