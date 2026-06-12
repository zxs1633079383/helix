//! 洪峰压测：NativeStorage 写吞吐（临时文件 SQLite，WAL）
//!
//! ## 测什么
//!
//! 1. `stress_batch_upsert_39x100`：单批 39 行 × 100 批（共 3900 行），
//!    走 `Storage::batch_upsert`（当前实现：逐行 INSERT，每行独立 autocommit）。
//! 2. `stress_batch_upsert_5000_single_batch`：5000 行单批。
//!    同时跑 **显式事务基线**（BEGIN + 逐行 INSERT + COMMIT）量化写放大倍数。
//! 3. `stress_monotonic_upsert_1000`：cursor 热路径（advance_cursor 每条 WS
//!    消息触发一次 monotonic_upsert，当前实现每次调用都先跑
//!    CREATE TABLE IF NOT EXISTS）。
//!
//! ## 跑法
//!
//! ```bash
//! cargo test --release -p helix-driver-native stress -- --nocapture --test-threads=1
//! ```
//!
//! 行 shape 与 helix-im event_to_upsert_op 一致：
//! (seq INTEGER PK, channel_id INTEGER, event_type INTEGER, raw TEXT ~150B)。

// 回归锚: E2（batch_upsert 写放大 9x→1x）—— 见 docs/过会方案/helix-架构评审-待办清单.md
use std::time::Instant;

use helix_core::effect::{MonotonicUpsertSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_native::NativeStorage;

// ─── 辅助 ────────────────────────────────────────────────────────────────────

/// 临时文件 SQLite（参考 engine_integration.rs 的建库方式）
async fn make_temp_storage(path: &str) -> NativeStorage {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open temp storage")
}

/// 建 messages 表（NativeStorage 池私有，用独立 sqlx 连接建表）
async fn create_messages_table(path: &str) {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("aux pool");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS messages \
         (seq INTEGER PRIMARY KEY NOT NULL, \
          channel_id INTEGER NOT NULL, \
          event_type INTEGER NOT NULL, \
          raw TEXT NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("create messages table");
    pool.close().await;
}

/// 构造一行（与 helix-im event_to_upsert_op 同 shape，raw 约 150 字节）
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
    let path = "/tmp/helix_stress_39x100.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(path).await;

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
    eprintln!("\n[stress_batch_upsert_39x100]");
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
}

// ─── 测试 2：5000 行单批 + 显式事务基线 ──────────────────────────────────────

#[tokio::test]
async fn stress_batch_upsert_5000_single_batch() {
    const N: i64 = 5_000;
    let path = "/tmp/helix_stress_5000.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(path).await;

    // A. 当前实现：batch_upsert（逐行 INSERT，每行独立 autocommit）
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
    let autocommit_elapsed = start.elapsed();

    // B. 基线：同样 5000 行包进一个显式事务（BEGIN..COMMIT，单次 fsync）
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("aux pool");
    sqlx::query("PRAGMA busy_timeout=5000")
        .execute(&pool)
        .await
        .unwrap();

    let rows_b: Vec<_> = (N + 1..=N * 2).map(make_row).collect();
    let start_tx = Instant::now();
    let mut tx = pool.begin().await.expect("begin tx");
    for row in &rows_b {
        let mut q = sqlx::query(
            "INSERT INTO messages (seq, channel_id, event_type, raw) VALUES (?, ?, ?, ?) \
             ON CONFLICT(seq) DO UPDATE SET channel_id = excluded.channel_id, \
             event_type = excluded.event_type, raw = excluded.raw",
        );
        for (_, v) in row {
            q = match v {
                SqlValue::Text(s) => q.bind(s.as_str()),
                SqlValue::Integer(i) => q.bind(*i),
                SqlValue::Real(f) => q.bind(*f),
                SqlValue::Null => q.bind(Option::<String>::None),
            };
        }
        q.execute(&mut *tx).await.expect("tx insert");
    }
    tx.commit().await.expect("commit");
    let tx_elapsed = start_tx.elapsed();
    pool.close().await;

    let amp = autocommit_elapsed.as_secs_f64() / tx_elapsed.as_secs_f64();
    eprintln!("\n[stress_batch_upsert_5000_single_batch]");
    eprintln!(
        "  A. batch_upsert (per-row autocommit) : {:.3} ms ({:.0} rows/s)",
        autocommit_elapsed.as_secs_f64() * 1e3,
        N as f64 / autocommit_elapsed.as_secs_f64()
    );
    eprintln!(
        "  B. explicit tx baseline (1 commit)   : {:.3} ms ({:.0} rows/s)",
        tx_elapsed.as_secs_f64() * 1e3,
        N as f64 / tx_elapsed.as_secs_f64()
    );
    eprintln!("  write amplification (A/B)            : {:.1}x", amp);
}

// ─── 测试 3：monotonic_upsert（cursor 热路径）× 1000 ─────────────────────────

#[tokio::test]
async fn stress_monotonic_upsert_1000() {
    const N: i64 = 1_000;
    let path = "/tmp/helix_stress_monotonic.db";
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

    eprintln!("\n[stress_monotonic_upsert_1000]");
    eprintln!(
        "  calls            : {} (每次含 CREATE TABLE IF NOT EXISTS + UPSERT)",
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
}
