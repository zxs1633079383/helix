use helix_core::effect::{GetSpec, MonotonicUpsertSpec, ScanSpec, SqlValue, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::PortOutcome;
use helix_driver_host::{
    execute_storage_ops, rows_from_reply_bytes, rows_to_reply_bytes, table_set_key, HostStorage,
};
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

async fn storage() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_host_conformance_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open storage");
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS messages \
             (id INTEGER PRIMARY KEY, content TEXT NOT NULL, seq INTEGER NOT NULL DEFAULT 0)",
        )
        .await
        .expect("create messages");
    storage
}

/// 文件 WAL 临时库 + messages 表。
///
/// 内存 `cache=shared` 库走 SQLite 共享缓存 **table-level lock**（WAL/MVCC 对 `:memory:`
/// 无效），并发读写同表会直接抛 `SQLITE_LOCKED`（不被 busy_timeout 重试）。读写分离的真
/// 并发只在**文件 WAL** 库下成立（生产路径），故并发 parity 测试必须用文件库。
async fn file_storage() -> (HostStorage, String) {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let path = format!(
        "/tmp/helix_host_conformance_{}_{}.db",
        std::process::id(),
        seq
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{path}-shm"));
    let _ = std::fs::remove_file(format!("{path}-wal"));
    let storage = HostStorage::open_sqlite_url(&format!("sqlite:{path}?mode=rwc"))
        .await
        .expect("open file storage");
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS messages \
             (id INTEGER PRIMARY KEY, content TEXT NOT NULL, seq INTEGER NOT NULL DEFAULT 0)",
        )
        .await
        .expect("create messages");
    (storage, path)
}

fn message_row(id: i64, content: &str, seq: i64) -> Vec<(String, SqlValue)> {
    vec![
        ("id".to_string(), SqlValue::Integer(id)),
        ("content".to_string(), SqlValue::Text(content.to_string())),
        ("seq".to_string(), SqlValue::Integer(seq)),
    ]
}

/// M2：BLOB 无损往返——非 UTF8 字节写入后按 `SqlValue::Blob` 原样读回（禁 from_utf8_lossy
/// 静默损坏），并验 codec(`rows_to_reply_bytes`/`rows_from_reply_bytes`) 往返同样无损。
#[tokio::test]
async fn host_storage_blob_roundtrip_lossless() {
    let storage = storage().await;
    storage
        .execute_raw("CREATE TABLE blobs (id INTEGER PRIMARY KEY, data BLOB NOT NULL)")
        .await
        .expect("create blobs");

    // 非法 UTF8 字节序列：from_utf8_lossy 会把它变成 U+FFFD 替换符（旧 M2 bug）
    let raw: Vec<u8> = vec![0xFF, 0x00, 0xFE, 0x80, 0x01];
    storage
        .batch_upsert(UpsertSpec {
            table: "blobs",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("data".to_string(), SqlValue::Blob(raw.clone())),
            ]],
            conflict_key: Some("id"),
        })
        .await
        .expect("insert blob");

    let got = storage
        .get(GetSpec {
            table: "blobs",
            key_col: "id",
            key_val: SqlValue::Integer(1),
        })
        .await
        .expect("get blob")
        .expect("row exists");

    match got.iter().find(|(k, _)| k == "data").map(|(_, v)| v) {
        Some(SqlValue::Blob(b)) => assert_eq!(b, &raw, "blob 字节必须无损"),
        other => panic!("data 应为 SqlValue::Blob，实际 {other:?}"),
    }

    // codec 往返也必须无损（数组编码，不退化成 Text）
    let bytes = rows_to_reply_bytes(&[got.clone()]);
    let back = rows_from_reply_bytes(&bytes).expect("decode");
    match back[0].iter().find(|(k, _)| k == "data").map(|(_, v)| v) {
        Some(SqlValue::Blob(b)) => assert_eq!(b, &raw, "codec 往返 blob 必须无损"),
        other => panic!("codec 往返后 data 应为 Blob，实际 {other:?}"),
    }
}

#[tokio::test]
async fn host_storage_matches_native_upsert_update_get_scan_semantics() {
    let storage = storage().await;

    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(1, "hello", 1)],
            conflict_key: Some("id"),
        })
        .await
        .expect("insert");

    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(1, "world", 2)],
            conflict_key: Some("id"),
        })
        .await
        .expect("conflict update");

    storage
        .batch_update(
            "messages",
            "id",
            &[SqlValue::Integer(1)],
            vec![("seq".to_string(), SqlValue::Integer(9))],
        )
        .await
        .expect("batch update");

    let row = storage
        .get(GetSpec {
            table: "messages",
            key_col: "id",
            key_val: SqlValue::Integer(1),
        })
        .await
        .expect("get")
        .expect("row exists");
    assert!(matches!(
        row.iter().find(|(k, _)| k == "content").map(|(_, v)| v),
        Some(SqlValue::Text(s)) if s == "world"
    ));
    assert!(matches!(
        row.iter().find(|(k, _)| k == "seq").map(|(_, v)| v),
        Some(SqlValue::Integer(9))
    ));

    let rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(1),
        })
        .await
        .expect("scan");
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn host_storage_creates_monotonic_table_at_open_and_uses_max_guard() {
    let storage = storage().await;

    let missing = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("missing".to_string()),
        })
        .await
        .expect("monotonic table exists");
    assert!(missing.is_none());

    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            scope_key: "channel:1".to_string(),
            value: 10,
        })
        .await
        .expect("first monotonic");
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            scope_key: "channel:1".to_string(),
            value: 3,
        })
        .await
        .expect("smaller monotonic");

    let row = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("channel:1".to_string()),
        })
        .await
        .expect("get monotonic")
        .expect("monotonic row");
    assert!(matches!(
        row.iter().find(|(k, _)| k == "value").map(|(_, v)| v),
        Some(SqlValue::Integer(10))
    ));
}

#[test]
fn row_reply_bytes_roundtrip_is_shared() {
    let rows = vec![vec![
        ("id".to_string(), SqlValue::Integer(1)),
        ("name".to_string(), SqlValue::Text("alice".to_string())),
        ("score".to_string(), SqlValue::Real(9.5)),
        ("extra".to_string(), SqlValue::Null),
    ]];

    let bytes = rows_to_reply_bytes(&rows);
    let restored = rows_from_reply_bytes(&bytes).expect("roundtrip");
    assert_eq!(restored.len(), 1);
    assert!(matches!(
        restored[0].iter().find(|(k, _)| k == "name").map(|(_, v)| v),
        Some(SqlValue::Text(s)) if s == "alice"
    ));
}

#[tokio::test]
async fn execute_storage_ops_returns_rows_bytes_and_short_circuits_on_error() {
    let storage = storage().await;
    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(7, "seven", 7)],
            conflict_key: Some("id"),
        })
        .await
        .expect("insert");

    let ok = execute_storage_ops(
        &storage,
        &[helix_core::effect::StorageOp::Get(GetSpec {
            table: "messages",
            key_col: "id",
            key_val: SqlValue::Integer(7),
        })],
    )
    .await;
    let PortOutcome::Ok(bytes) = ok else {
        panic!("get should return ok rows bytes");
    };
    let rows = rows_from_reply_bytes(&bytes.0).expect("rows json");
    assert_eq!(rows.len(), 1);

    let err = execute_storage_ops(
        &storage,
        &[
            helix_core::effect::StorageOp::Get(GetSpec {
                table: "missing_table",
                key_col: "id",
                key_val: SqlValue::Integer(7),
            }),
            helix_core::effect::StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                scope_key: "must_not_run".to_string(),
                value: 1,
            }),
        ],
    )
    .await;
    assert!(matches!(err, PortOutcome::Err(_)));

    let missing = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("must_not_run".to_string()),
        })
        .await
        .expect("query monotonic");
    assert!(missing.is_none(), "second op must not run after error");
}

// ─── B1 回归守护：读写分离并发 parity ────────────────────────────────────────
//
// 这是能抓住 B1（三池塌成单 Mutex → 全串行）的关键测试：spawn K 个 task 交错写
// 不同表 + 并发 get/scan，断言 ① 不死锁 ② 墙钟有界 ③ 结果正确。
// 若回退成单把锁，读写互斥串行 → 墙钟显著拉长（在慢盘/CI 上可能超时），且任何
// 读阻塞写、写阻塞读的回归会被结果错误捕捉。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_writes_and_reads_do_not_deadlock_and_stay_correct() {
    use std::time::{Duration, Instant};

    // 文件 WAL 库：读写分离真并发的成立前提（见 file_storage 注释）。
    let (storage, _path) = file_storage().await;
    // 第二张表，验证「不同表交错写」走写连接仍正确。
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS notes \
             (id INTEGER PRIMARY KEY, body TEXT NOT NULL)",
        )
        .await
        .expect("create notes");

    const K: i64 = 32;
    let start = Instant::now();

    // 并发写 messages（不同 id）
    let mut handles = Vec::new();
    for i in 1..=K {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            s.batch_upsert(UpsertSpec {
                table: "messages",
                rows: vec![message_row(i, &format!("msg-{i}"), i)],
                conflict_key: Some("id"),
            })
            .await
            .expect("concurrent message upsert");
        }));
    }
    // 并发写 notes（不同表，交错）
    for i in 1..=K {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            s.batch_upsert(UpsertSpec {
                table: "notes",
                rows: vec![vec![
                    ("id".to_string(), SqlValue::Integer(i)),
                    ("body".to_string(), SqlValue::Text(format!("note-{i}"))),
                ]],
                conflict_key: Some("id"),
            })
            .await
            .expect("concurrent note upsert");
        }));
    }
    // 并发读（get/scan）与写交错进行，验证读不阻死写、池可并发借还
    for _ in 0..K {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            let _ = s
                .scan(ScanSpec {
                    table: "messages",
                    limit: Some(100),
                })
                .await
                .expect("concurrent scan");
            let _ = s
                .get(GetSpec {
                    table: "messages",
                    key_col: "id",
                    key_val: SqlValue::Integer(1),
                })
                .await
                .expect("concurrent get");
        }));
    }

    // 不死锁：全部 join 在合理墙钟内完成。
    for h in handles {
        h.await.expect("task join");
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(20),
        "concurrent workload took too long ({elapsed:?}) — possible serialization regression"
    );

    // 结果正确：messages 与 notes 各 K 行全部落盘。
    let msgs = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(1000),
        })
        .await
        .expect("final scan messages");
    assert_eq!(
        msgs.len(),
        K as usize,
        "all concurrent message writes landed"
    );
    let notes = storage
        .scan(ScanSpec {
            table: "notes",
            limit: Some(1000),
        })
        .await
        .expect("final scan notes");
    assert_eq!(notes.len(), K as usize, "all concurrent note writes landed");
}

// ─── E5 守护：monotonic 等值写是 no-op（MAX guard 仅严格大于才前进）────────────
#[tokio::test]
async fn monotonic_upsert_equal_value_is_noop() {
    let storage = storage().await;
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            scope_key: "ch:eq".to_string(),
            value: 10,
        })
        .await
        .expect("first 10");
    // 再写同值 10 —— WHERE excluded.value > value 不成立 → no-op，仍是 10。
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            scope_key: "ch:eq".to_string(),
            value: 10,
        })
        .await
        .expect("second 10 no-op");

    let row = storage
        .get(GetSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            key_val: SqlValue::Text("ch:eq".to_string()),
        })
        .await
        .expect("get monotonic")
        .expect("monotonic row");
    assert!(matches!(
        row.iter().find(|(k, _)| k == "value").map(|(_, v)| v),
        Some(SqlValue::Integer(10))
    ));
}

// ─── E2 守护：batch_upsert 中途失败整批回滚（事务原子性）──────────────────────
#[tokio::test]
async fn batch_upsert_rolls_back_entire_batch_on_midway_failure() {
    let storage = storage().await;
    // 加 NOT NULL 约束的列，构造一行违反约束触发中途失败。
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS strict_tbl \
             (id INTEGER PRIMARY KEY, val TEXT NOT NULL)",
        )
        .await
        .expect("create strict table");

    // 批中第 2 行 val=Null 违反 NOT NULL → 事务中途失败 → 整批回滚（含第 1 行）。
    let err = storage
        .batch_upsert(UpsertSpec {
            table: "strict_tbl",
            rows: vec![
                vec![
                    ("id".to_string(), SqlValue::Integer(1)),
                    ("val".to_string(), SqlValue::Text("ok".to_string())),
                ],
                vec![
                    ("id".to_string(), SqlValue::Integer(2)),
                    ("val".to_string(), SqlValue::Null),
                ],
            ],
            conflict_key: Some("id"),
        })
        .await;
    assert!(err.is_err(), "batch with NOT NULL violation must fail");

    // 第 1 行也不得落盘（整批回滚）。
    let landed = storage
        .scan(ScanSpec {
            table: "strict_tbl",
            limit: Some(100),
        })
        .await
        .expect("scan strict_tbl");
    assert_eq!(
        landed.len(),
        0,
        "no rows should land — entire batch rolled back"
    );
}

#[test]
fn table_set_key_is_sorted_and_deduplicated() {
    use helix_core::effect::{MonotonicUpsertSpec, StorageOp};

    let key = table_set_key(&[
        StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
        }),
        StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
            scope_key: "channel:1".to_string(),
            value: 1,
        }),
        StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
        }),
    ]);

    assert_eq!(key, "_helix_monotonic|messages");
}
