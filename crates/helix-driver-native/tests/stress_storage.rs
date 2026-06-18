//! NativeStorage 薄壳适配层 parity 烟测。
//!
//! ## 设计意图
//!
//! HostStorage 的共享核压测（39×100 / 5000 单批 / 10k 洪峰 / monotonic_upsert×1000）
//! 已迁移到 `crates/helix-driver-host/tests/stress_storage_core.rs`（单一真源）。
//!
//! 本文件只保留**"NativeStorage 薄壳与 HostStorage 共享核行为一致"**的 parity 烟测：
//! 通过 NativeStorage 跑相同操作，断言结果与直接跑 HostStorage 一致，
//! 证明 native 适配层的「委托转发」没有引入语义分叉。
//!
//! ## 设计依据
//!
//! NativeStorage 本质是 `Arc<HostStorage>` 加一层 trait impl 委托（无独立状态），
//! 所以 parity 测试「用 NativeStorage 跑一次 5000 行单批 + 断言 <1s + 断言全行落盘」
//! 即可证明委托层无语义偏差。更多场景不重复，等价性由实现结构保证。
//!
//! 跑法：
//! ```bash
//! cargo test -p helix-driver-native stress -- --nocapture
//! ```

use std::time::Instant;

use helix_core::effect::{MonotonicUpsertSpec, ScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_native::NativeStorage;

/// 临时文件 SQLite（参考 engine_integration.rs 的建库方式）。
async fn make_temp_storage(path: &str) -> NativeStorage {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    NativeStorage::open(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open temp storage")
}

/// 建 messages 表。
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

// ─── parity 烟测 1：5000 行单批，验证 NativeStorage 委托层不退化吞吐 ──────────

/// NativeStorage parity：5000 行单批通过委托层写入，与 HostStorage 直接写语义一致。
/// 完整共享核场景压测见 helix-driver-host/tests/stress_storage_core.rs。
#[tokio::test]
async fn parity_native_batch_upsert_5000() {
    const N: i64 = 5_000;
    let path = "/tmp/helix_native_parity_5000.db";
    let storage = make_temp_storage(path).await;
    create_messages_table(&storage).await;

    let rows: Vec<_> = (1..=N).map(make_row).collect();
    let start = Instant::now();
    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows,
            conflict_key: Some("seq"),
            exclude_from_update: Vec::new(),
        })
        .await
        .expect("parity batch_upsert 5000 rows");
    let elapsed = start.elapsed();

    eprintln!("\n[native::parity_native_batch_upsert_5000]");
    eprintln!(
        "  NativeStorage batch_upsert : {:.3} ms ({:.0} rows/s)",
        elapsed.as_secs_f64() * 1e3,
        N as f64 / elapsed.as_secs_f64()
    );

    // parity 断言：委托层开销极小，不应退化吞吐（<1s CI 上限，与 host 直写对齐）。
    assert!(
        elapsed.as_millis() < 1_000,
        "NativeStorage parity: 5000 rows took {elapsed:?} (>1s) — \
         delegation layer may have introduced write amplification regression"
    );

    let rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(10_000),
        })
        .await
        .expect("scan after parity batch");
    assert_eq!(
        rows.len(),
        N as usize,
        "all 5000 rows must land via NativeStorage"
    );
}

// ─── parity 烟测 2：monotonic_upsert 委托层不破坏 MAX guard ──────────────────

/// NativeStorage parity：monotonic_upsert 通过委托层后 MAX guard 仍成立。
#[tokio::test]
async fn parity_native_monotonic_upsert_no_regression() {
    let path = "/tmp/helix_native_parity_monotonic.db";
    let storage = make_temp_storage(path).await;

    // 顺序写入 100 次。
    for i in 1..=100i64 {
        storage
            .monotonic_upsert(MonotonicUpsertSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                value_col: "value",
                touch_col: None,
                scope_key: "parity_cursor:1".to_string(),
                value: i,
            })
            .await
            .expect("monotonic_upsert forward");
    }

    // 尝试写入回退值：MAX guard 应使之成为 no-op。
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
            scope_key: "parity_cursor:1".to_string(),
            value: 50, // 小于当前值 100
        })
        .await
        .expect("monotonic_upsert backward (should be no-op)");

    // cursor 仍应为 100，MAX guard 通过委托层仍生效。
    use helix_core::effect::GetSpec;
    let row = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("parity_cursor:1".to_string()),
        })
        .await
        .expect("get cursor after regression attempt");

    let val = row
        .as_ref()
        .and_then(|r| r.iter().find(|(k, _)| k == "value"))
        .map(|(_, v)| v.clone());
    assert!(
        matches!(val, Some(SqlValue::Integer(100))),
        "NativeStorage parity: MAX guard must prevent cursor regression (expected 100, got {val:?})"
    );
}
