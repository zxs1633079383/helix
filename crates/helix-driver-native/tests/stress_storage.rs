//! 洪峰压测：NativeStorage 写吞吐（临时文件 SQLite，WAL）
//!
//! ## 测什么
//!
//! 1. `stress_batch_upsert_39x100`：单批 39 行 × 100 批（共 3900 行），
//!    走 shared host `Storage::batch_upsert`。
//! 2. `stress_batch_upsert_5000_single_batch`：5000 行单批，验证 shared host
//!    storage 保持事务包裹写入。
//! 3. `stress_monotonic_upsert_1000`：cursor 热路径（advance_cursor 每条 WS
//!    消息触发一次 monotonic_upsert）。
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

/// 建 messages 表（经 NativeStorage 走 shared host storage）。
async fn create_messages_table(storage: &NativeStorage) {
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

    eprintln!("\n[stress_batch_upsert_5000_single_batch]");
    eprintln!(
        "  shared host batch_upsert             : {:.3} ms ({:.0} rows/s)",
        elapsed.as_secs_f64() * 1e3,
        N as f64 / elapsed.as_secs_f64()
    );
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
}
