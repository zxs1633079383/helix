# helix-driver-native 局部指令（就近优先，全局见根 CLAUDE.md）

## 本 crate 契约（engine_loop 五不变量，破坏须同步改文档）

1. **回灌必达**：PortReply 走独立 unbounded reply_tx，永不堵塞写 worker
2. **主循环不等磁盘**：Persist 入有界 job queue（cap=1024）即返回；队列满时 send().await 把背压传导回 tick_tx
3. **同 key 写顺序**：per-table-set 长驻 worker 单消费者保序；多表 ops 走组合键独立 worker（跨 key 不保证相对顺序——需有序就合进同一 Persist）
4. **回灌优先**：biased select reply_rx 在前
5. **graceful drain**：「run_engine_loop 返回 = 全部已接收 Persist 已落盘」（drop senders → join workers → drain replies）

## Storage 约定

- 三池：sync 写(1) / 用户写(1) / 读(4)，共享 WAL；batch_upsert 整批一事务（E2，写放大 ~1.0x）
- 本 crate 是 SystemTime/线程等 ambient API 的**唯一合法落点**（clippy 在此 allow）
- rows→bytes JSON 约定的定义点在 engine_loop.rs（helix-im 反序列化端必须对齐）

## 改动验收

```bash
cargo test -p helix-driver-native
cargo test --release -p helix-driver-native --test stress_engine --test stress_storage -- --test-threads=1   # 改泵/存储时必跑
```
