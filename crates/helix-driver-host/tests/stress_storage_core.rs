//! host 共享核存储压测（从 helix-driver-native/tests/stress_storage.rs 下沉）。
//!
//! ## 为什么在 helix-driver-host
//!
//! `batch_upsert` / `monotonic_upsert` / 并发读写的语义实现在 `HostStorage`（单一真源）。
//! native / ffi 两端复用同一实现，测「共享核行为」理应落在 host 层，
//! native 只留「NativeStorage 薄壳适配层 parity 烟测」（见 helix-driver-native 同名文件）。
//!
//! ## 测什么
//!
//! 1. `stress_batch_upsert_39x100`：单批 39 行 × 100 批（共 3900 行），事务完整性 + 吞吐。
//! 2. `stress_batch_upsert_5000_single_batch`：5000 行单批，整批一事务，回归写放大 ~1x。
//! 3. `stress_10k_flood_no_deadlock_all_landed`：100 并发写 + 32 并发读，不死锁 + 全落盘。
//! 4. `stress_monotonic_upsert_1000`：cursor 热路径（每条 WS 消息触发 1 次 monotonic_upsert）。
//!
//! 跑法：
//! ```bash
//! cargo test -p helix-driver-host stress_storage -- --nocapture --test-threads=1
//! ```

use std::time::Instant;

use helix_core::effect::{GetSpec, MonotonicUpsertSpec, ScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;

// ─── 辅助 ────────────────────────────────────────────────────────────────────

/// 临时文件 WAL 库（测真并发必须文件库，见 storage_conformance.rs 头注说明）。
async fn make_temp_storage(path: &str) -> HostStorage {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    HostStorage::open_sqlite_url(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open temp storage")
}

/// 建 messages 表（与 helix-im event_to_upsert_op 同 shape）。
async fn create_messages_table(storage: &HostStorage) {
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS messages \
         (seq INTEGER PRIMARY KEY NOT NULL, \
          channel_id INTEGER NOT NULL, \
          event_type INTEGER NOT NULL, \
          raw TEXT NOT NULL)",
        )
        .await
        .expect("create messages table");
}

/// 构造一行（raw 约 150 字节，对齐 helix-im event_to_upsert_op shape）。
fn make_row(seq: i64) -> Vec<(String, SqlValue)> {
    let raw = format!(
        "{{\"event\":\"posted\",\"data\":{{\"channel_id\":1,\"seq\":{seq},\
         \"event_type\":1,\"msg_id\":\"msg_{seq}\",\"actor_id\":\"user_1\",\
         \"created_at\":1000000,\"payload\":null}}}}"
    );
    vec![
        ("seq".to_string(), SqlValue::Integer(seq)),
        ("channel_id".to_string(), SqlValue::Integer(1)),
        ("event_type".to_string(), SqlValue::Integer(1)),
        ("raw".to_string(), SqlValue::Text(raw)),
    ]
}

// ─── 测试 1：39 行 × 100 批 ──────────────────────────────────────────────────

#[tokio::test]
async fn stress_batch_upsert_39x100() {
    const ROWS_PER_BATCH: i64 = 39;
    const BATCHES: i64 = 100;
    let path = "/tmp/helix_host_stress_39x100.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(&storage).await;

    let start = Instant::now();
    for b in 0..BATCHES {
        let rows: Vec<_> = (0..ROWS_PER_BATCH)
            .map(|i| make_row(b * ROWS_PER_BATCH + i + 1))
            .collect();
        storage
            .batch_upsert(UpsertSpec {
                table: "messages",
                rows,
                conflict_key: Some("seq"),
            })
            .await
            .expect("batch_upsert 39 rows");
    }
    let elapsed = start.elapsed();

    let total_rows = ROWS_PER_BATCH * BATCHES;
    eprintln!("\n[host::stress_batch_upsert_39x100]");
    eprintln!(
        "  batches          : {} x {} rows = {} rows",
        BATCHES, ROWS_PER_BATCH, total_rows
    );
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!(
        "  per-batch latency: {:.3} ms",
        elapsed.as_secs_f64() * 1e3 / BATCHES as f64
    );
    eprintln!(
        "  row throughput   : {:.0} rows/s",
        total_rows as f64 / elapsed.as_secs_f64()
    );

    // 事务完整性断言（HX-C011：破坏即 fail）——全 3900 行须落盘。
    // 若某批 batch_upsert 静默丢行 / 事务未提交，scan 回来 < total_rows → FAIL。
    let landed = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(10_000),
        })
        .await
        .expect("scan after 39x100");
    assert_eq!(
        landed.len(),
        total_rows as usize,
        "全 {total_rows} 行须落盘（39×100 批事务完整性）"
    );
}

// ─── 测试 2：5000 行单批 ──────────────────────────────────────────────────────

/// 回归锚：E2（batch_upsert 写放大 9x→1x）——5000 行单事务应在 ~100ms 量级内完成。
#[tokio::test]
async fn stress_batch_upsert_5000_single_batch() {
    const N: i64 = 5_000;
    let path = "/tmp/helix_host_stress_5000.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(&storage).await;

    let rows: Vec<_> = (1..=N).map(make_row).collect();
    let start = Instant::now();
    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows,
            conflict_key: Some("seq"),
        })
        .await
        .expect("batch_upsert 5000 rows");
    let elapsed = start.elapsed();

    eprintln!("\n[host::stress_batch_upsert_5000_single_batch]");
    eprintln!(
        "  host batch_upsert          : {:.3} ms ({:.0} rows/s)",
        elapsed.as_secs_f64() * 1e3,
        N as f64 / elapsed.as_secs_f64()
    );

    // 回归断言：5000 行单事务应在 1s 以内（CI 上限）。
    // 退化为逐行独立事务（写放大 9x）会跳到数秒级 → 被此断言捕捉。
    assert!(
        elapsed.as_millis() < 1_000,
        "batch_upsert 5000 rows took {elapsed:?} (>1s) — write amplification regression \
         (expected single-transaction ~100ms order)"
    );

    // 全部落盘（单事务原子提交）。
    let rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(10_000),
        })
        .await
        .expect("scan after 5000 batch");
    assert_eq!(rows.len(), N as usize, "all 5000 rows must land");
}

// ─── 测试 3：10k 并发灌 + 读写交错 ───────────────────────────────────────────

/// 不死锁 + 全落盘：100 并发写 batch + 32 并发读，验证 WAL 读写不互斥。
/// 注：文件 WAL 库下读写真并发（storage_conformance.rs §2.2）；内存库 cache=shared 无此效果。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_10k_flood_no_deadlock_all_landed() {
    use std::time::Duration;

    const BATCHES: i64 = 100;
    const PER_BATCH: i64 = 100;
    const TOTAL: i64 = BATCHES * PER_BATCH;
    let path = "/tmp/helix_host_stress_10k_flood.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(&storage).await;

    let start = Instant::now();

    // 并发灌 100 批，每批 100 行（不同 seq 段，互不冲突）。
    let mut handles = Vec::new();
    for b in 0..BATCHES {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            let rows: Vec<_> = (0..PER_BATCH)
                .map(|i| make_row(b * PER_BATCH + i + 1))
                .collect();
            s.batch_upsert(UpsertSpec {
                table: "messages",
                rows,
                conflict_key: Some("seq"),
            })
            .await
            .expect("flood batch_upsert");
        }));
    }
    // 交错并发读，验证读不阻死写（WAL 读写分离）。
    for _ in 0..32 {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            let _ = s
                .scan(ScanSpec {
                    table: "messages",
                    limit: Some(10_000),
                })
                .await
                .expect("flood scan");
        }));
    }

    for h in handles {
        h.await.expect("flood task join");
    }
    let elapsed = start.elapsed();

    eprintln!("\n[host::stress_10k_flood_no_deadlock_all_landed]");
    eprintln!("  total rows       : {TOTAL}");
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);

    // 不死锁：有界墙钟。
    assert!(
        elapsed < Duration::from_secs(30),
        "10k flood took {elapsed:?} (>30s) — possible deadlock/serialization regression"
    );

    // 全落盘：所有 10k 行可读回。
    let rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(20_000),
        })
        .await
        .expect("final scan after flood");
    assert_eq!(rows.len(), TOTAL as usize, "all 10k flooded rows must land");
}

// ─── 测试 4：monotonic_upsert（cursor 热路径）× 1000 ─────────────────────────

#[tokio::test]
async fn stress_monotonic_upsert_1000() {
    const N: i64 = 1_000;
    let path = "/tmp/helix_host_stress_monotonic.db";
    let storage = make_temp_storage(path).await;

    let start = Instant::now();
    for i in 1..=N {
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                scope_key: "channel_event_cursor:1".to_string(),
                value: i,
            })
            .await
            .expect("monotonic_upsert");
    }
    let elapsed = start.elapsed();

    eprintln!("\n[host::stress_monotonic_upsert_1000]");
    eprintln!(
        "  calls            : {} (UPSERT only; schema created at open)",
        N
    );
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);
    eprintln!(
        "  per-call latency : {:.3} ms",
        elapsed.as_secs_f64() * 1e3 / N as f64
    );
    eprintln!(
        "  call throughput  : {:.0} calls/s",
        N as f64 / elapsed.as_secs_f64()
    );

    // 单调防回退断言（HX-C011：破坏即 fail）——MAX guard 验证。
    // 写完 1..N 终态应为 N；再写一个回退值（N/2 < N），MAX guard 应拒绝，终态仍 N。
    // 若 guard 被移除（无条件 UPDATE），回退值覆写 → 下面断言 FAIL。
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            scope_key: "channel_event_cursor:1".to_string(),
            value: N / 2,
        })
        .await
        .expect("regression monotonic_upsert");
    let row = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("channel_event_cursor:1".to_string()),
        })
        .await
        .expect("get monotonic after regression");
    let val = row
        .as_ref()
        .and_then(|r| r.iter().find(|(k, _)| k == "value"))
        .map(|(_, v)| v.clone());
    assert!(
        matches!(val, Some(SqlValue::Integer(v)) if v == N),
        "monotonic cursor 终态应为 {N}（回退值 {} 被 MAX guard 拒），实际: {val:?}",
        N / 2
    );
}
