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

fn message_row(id: i64, content: &str, seq: i64) -> Vec<(String, SqlValue)> {
    vec![
        ("id".to_string(), SqlValue::Integer(id)),
        ("content".to_string(), SqlValue::Text(content.to_string())),
        ("seq".to_string(), SqlValue::Integer(seq)),
    ]
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
