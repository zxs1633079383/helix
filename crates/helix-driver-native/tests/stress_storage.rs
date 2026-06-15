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

use helix_core::effect::{MonotonicUpsertSpec, ScanSpec, SqlValue, UpsertSpec};
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

    // 回归断言（E2 写放大 1x 基线，被 a0595607 删掉的吞吐基线恢复）：
    // 5000 行单事务应在 ~100ms 量级内完成。放宽到 1s 作 CI 上限（慢盘/竞争容差），
    // 真正能抓住的是「写放大回退到 9x / 退化成逐行独立事务」会让墙钟跳到数秒级。
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

// ─── 测试 4：10k 裸灌 + graceful drain 全落盘 ─────────────────────────────────
//
// 恢复被 a0595607 删掉的「10k 裸灌不死锁 + drain 后全落盘」基线。
// 这里直接经 Storage::batch_upsert 灌（每批 100 行 × 100 批 = 10k），并发交错读，
// 断言 ① 不死锁（有界墙钟内完成）② 全部 10k 行落盘。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stress_10k_flood_no_deadlock_all_landed() {
    use std::time::Duration;

    const BATCHES: i64 = 100;
    const PER_BATCH: i64 = 100;
    const TOTAL: i64 = BATCHES * PER_BATCH;
    let path = "/tmp/helix_stress_10k_flood.db";
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
    // 交错并发读，验证读不阻死写（读写分离）。
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

    eprintln!("\n[stress_10k_flood_no_deadlock_all_landed]");
    eprintln!("  total rows       : {TOTAL}");
    eprintln!("  total time       : {:.3} ms", elapsed.as_secs_f64() * 1e3);

    // 不死锁：有界墙钟。
    assert!(
        elapsed < Duration::from_secs(30),
        "10k flood took {elapsed:?} (>30s) — possible deadlock/serialization regression"
    );

    // graceful drain 后全落盘：所有 10k 行可读回。
    let rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(20_000),
        })
        .await
        .expect("final scan after flood");
    assert_eq!(rows.len(), TOTAL as usize, "all 10k flooded rows must land");
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
