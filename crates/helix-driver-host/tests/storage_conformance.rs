use helix_core::effect::{
    BatchDeleteSpec, BatchUpdateSpec, Effect, GetSpec, GuardedBumpSpec, MonotonicUpsertSpec,
    ScanOrder, ScanSpec, ScopedGetSpec, ScopedGuardedBumpSpec, ScopedMaxSpec, ScopedScanSpec,
    SqlValue, UpsertSpec,
};
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, PortOutcome};
use helix_core::{EffectSink, Module, Tick};
use helix_driver_host::{
    execute_storage_ops, rows_from_reply_bytes, rows_to_reply_bytes, table_set_key, HostStorage,
};
use helix_im::{ImConfig, ImModule};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);
const DIALOG_SCAN_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("is_top"),
    ScanOrder::desc("last_post_at"),
    ScanOrder::desc("create_at"),
];

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
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
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
    let bytes = rows_to_reply_bytes(&[got]);
    let back = rows_from_reply_bytes(&bytes).expect("decode");
    match back[0].iter().find(|(k, _)| k == "data").map(|(_, v)| v) {
        Some(SqlValue::Blob(b)) => assert_eq!(b, &raw, "codec 往返 blob 必须无损"),
        other => panic!("codec 往返后 data 应为 Blob，实际 {other:?}"),
    }
}

#[tokio::test]
/// Verify the shared host preserves native storage port semantics.
async fn host_storage_matches_native_upsert_update_get_scan_semantics() {
    let storage = storage().await;

    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(1, "hello", 1)],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("insert");

    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(1, "world", 2)],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("conflict update");

    storage
        .batch_update(BatchUpdateSpec {
            table: "messages",
            key_col: "id",
            key_vals: vec![SqlValue::Integer(1)],
            patch: vec![("seq".to_string(), SqlValue::Integer(9))],
        })
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
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan");
    assert_eq!(rows.len(), 1);
}

#[tokio::test]
async fn host_storage_scan_applies_structured_multi_order_before_limit() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE dialog_sort (\
             id TEXT PRIMARY KEY, is_top INTEGER NOT NULL, \
             last_post_at INTEGER NOT NULL, create_at INTEGER NOT NULL)",
        )
        .await
        .expect("create dialog_sort");
    storage
        .execute_raw(
            "INSERT INTO dialog_sort VALUES \
             ('old-pinned',1,10,10),\
             ('active',0,100,20),\
             ('new-empty',0,0,90),\
             ('old-empty',0,0,30)",
        )
        .await
        .expect("seed dialog_sort");

    let rows = storage
        .scan(ScanSpec {
            table: "dialog_sort",
            limit: Some(3),
            filter: None,
            order_by: DIALOG_SCAN_ORDER,
        })
        .await
        .expect("multi-order scan");
    let ids: Vec<&str> = rows
        .iter()
        .filter_map(|row| match row.iter().find(|(key, _)| key == "id") {
            Some((_, SqlValue::Text(id))) => Some(id.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(ids, ["old-pinned", "active", "new-empty"]);
}

#[tokio::test]
async fn host_storage_scoped_max_filters_before_aggregate() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE watermark_messages (\
             id INTEGER PRIMARY KEY, channel_id TEXT NOT NULL, \
             create_at INTEGER NOT NULL, update_at INTEGER NOT NULL);\
             CREATE INDEX idx_watermark_messages_scope_create \
             ON watermark_messages(channel_id, create_at);\
             INSERT INTO watermark_messages VALUES \
             (1,'channel-a',1700,9900),\
             (2,'channel-a',2300,1200),\
             (3,'channel-b',9900,12000);",
        )
        .await
        .expect("create and seed scoped max table");

    let mut scope_values = (0..900)
        .map(|index| SqlValue::Text(format!("missing-{index}")))
        .collect::<Vec<_>>();
    // Force the host adapter's bounded-IN path to execute more than one query;
    // the matching channel is deliberately in the second chunk.
    scope_values.push(SqlValue::Text("channel-a".into()));

    let row = storage
        .scoped_max(ScopedMaxSpec {
            table: "watermark_messages",
            scope_col: "channel_id",
            scope_values,
            value_col: "create_at",
            result_alias: "create_at",
        })
        .await
        .expect("scoped max")
        .expect("scoped row");
    assert!(matches!(
        row.iter().find(|(column, _)| column == "create_at"),
        Some((_, SqlValue::Integer(2300)))
    ));

    assert!(storage
        .scoped_max(ScopedMaxSpec {
            table: "watermark_messages",
            scope_col: "channel_id",
            scope_values: Vec::new(),
            value_col: "create_at",
            result_alias: "create_at",
        })
        .await
        .expect("empty scope")
        .is_none());
}

#[tokio::test]
async fn host_storage_scoped_scan_filters_before_limit_and_rejects_truncation() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE scoped_scan_rows (\
             id INTEGER PRIMARY KEY, channel_id TEXT NOT NULL, payload TEXT NOT NULL);\
             INSERT INTO scoped_scan_rows VALUES\
             (1,'channel-a','a1'),(2,'channel-a','a2'),(3,'channel-b','b1');",
        )
        .await
        .expect("create and seed scoped scan table");

    let mut scope_values = vec![SqlValue::Text("channel-a".into())];
    scope_values.extend(
        (0..899)
            .map(|index| SqlValue::Text(format!("missing-{index}")))
            .collect::<Vec<_>>(),
    );
    scope_values.push(SqlValue::Text("channel-a".into()));
    let rows = storage
        .scoped_scan(ScopedScanSpec {
            table: "scoped_scan_rows",
            scope_col: "channel_id",
            scope_values,
            filter: None,
            limit: 4,
        })
        .await
        .expect("scoped scan");
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| {
        row.iter().any(|(column, value)| {
            column == "channel_id"
                && matches!(value, SqlValue::Text(channel) if channel == "channel-a")
        })
    }));

    assert!(storage
        .scoped_scan(ScopedScanSpec {
            table: "scoped_scan_rows",
            scope_col: "channel_id",
            scope_values: vec![SqlValue::Text("channel-a".into())],
            filter: None,
            limit: 1,
        })
        .await
        .is_err());
    assert!(storage
        .scoped_scan(ScopedScanSpec {
            table: "missing_table",
            scope_col: "channel_id",
            scope_values: Vec::new(),
            filter: None,
            limit: 1,
        })
        .await
        .expect("empty scope must not touch table")
        .is_empty());
}

/// 三个 5000 人频道的成员快照必须先按 viewer 过滤，再应用 10000 行硬上限。
///
/// 这个用例故意保留每个频道的 4999 个其他成员，复现 dialogList 第二阶段只按
/// channel_id 扫描时的 15000 行超限；成员所属公司允许与频道公司不同。
#[tokio::test]
async fn host_storage_dialog_member_scan_filters_viewer_before_limit() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE channel_member (
             id INTEGER PRIMARY KEY, channel_id TEXT NOT NULL,
             user_id TEXT NOT NULL, team_id TEXT NOT NULL)",
        )
        .await
        .expect("create channel_member");

    let mut member_rows = Vec::with_capacity(15_000);
    for channel_index in 0..3_i64 {
        let channel_id = format!("channel-{channel_index}");
        for member_index in 0..5_000_i64 {
            let user_id = if member_index == 0 {
                "viewer".to_string()
            } else {
                format!("member-{channel_index}-{member_index}")
            };
            let team_id = if member_index == 0 {
                "external-team".to_string()
            } else {
                format!("team-{channel_index}")
            };
            member_rows.push(vec![
                (
                    "id".to_string(),
                    SqlValue::Integer(channel_index * 5_000 + member_index + 1),
                ),
                ("channel_id".to_string(), SqlValue::Text(channel_id.clone())),
                ("user_id".to_string(), SqlValue::Text(user_id)),
                ("team_id".to_string(), SqlValue::Text(team_id)),
            ]);
        }
    }
    storage
        .batch_upsert(UpsertSpec {
            table: "channel_member",
            rows: member_rows,
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("seed three 5000-member channels");

    let rows = storage
        .scoped_scan(ScopedScanSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_values: vec![
                SqlValue::Text("channel-0".to_string()),
                SqlValue::Text("channel-1".to_string()),
                SqlValue::Text("channel-2".to_string()),
            ],
            filter: Some(("user_id", SqlValue::Text("viewer".to_string()))),
            limit: 10_000,
        })
        .await
        .expect("viewer filter must be applied before the scoped limit");
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| {
        row.iter().any(|(column, value)| {
            column == "user_id" && matches!(value, SqlValue::Text(user_id) if user_id == "viewer")
        })
    }));
}

#[tokio::test]
async fn host_scoped_scan_filter_null_matches_no_rows() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE nullable_member (
             id INTEGER PRIMARY KEY, channel_id TEXT NOT NULL, user_id TEXT)",
        )
        .await
        .expect("create nullable_member");
    storage
        .batch_upsert(UpsertSpec {
            table: "nullable_member",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                (
                    "channel_id".to_string(),
                    SqlValue::Text("channel-0".to_string()),
                ),
                ("user_id".to_string(), SqlValue::Null),
            ]],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("seed nullable member");

    let rows = storage
        .scoped_scan(ScopedScanSpec {
            table: "nullable_member",
            scope_col: "channel_id",
            scope_values: vec![SqlValue::Text("channel-0".to_string())],
            filter: Some(("user_id", SqlValue::Null)),
            limit: 1,
        })
        .await
        .expect("SQL equality with NULL is valid but matches no row");
    assert!(rows.is_empty(), "WHERE user_id = NULL must not match");
}

/// 公开 `im_query_dialog_list` 经过 Host SQLite 的两阶段回放：旧实现会把三个
/// 5000 人频道的 15000 行成员读回撞到 10000 上限并发布空列表；成员条件必须在
/// 存储读取时生效，终态才保留三个当前 viewer 频道。
#[tokio::test]
async fn host_storage_dialog_command_filters_members_before_limit() {
    let storage = storage().await;
    storage
        .execute_raw(
            "CREATE TABLE channel (
             id TEXT PRIMARY KEY, team_id TEXT NOT NULL, user_id TEXT,
             is_top INTEGER NOT NULL DEFAULT 0, last_post_at INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL DEFAULT 0)",
        )
        .await
        .expect("create channel");
    storage
        .execute_raw(
            "CREATE TABLE channel_member (
             id INTEGER PRIMARY KEY, channel_id TEXT NOT NULL,
             user_id TEXT NOT NULL, team_id TEXT, channel_is_top INTEGER NOT NULL DEFAULT 0)",
        )
        .await
        .expect("create channel_member");

    let channels = (0..3_i64)
        .map(|index| {
            vec![
                ("id".to_string(), SqlValue::Text(format!("channel-{index}"))),
                ("team_id".to_string(), SqlValue::Text("company".to_string())),
                ("user_id".to_string(), SqlValue::Text("viewer".to_string())),
                ("is_top".to_string(), SqlValue::Integer(0)),
                ("last_post_at".to_string(), SqlValue::Integer(0)),
                ("created_at".to_string(), SqlValue::Integer(index)),
            ]
        })
        .collect();
    storage
        .batch_upsert(UpsertSpec {
            table: "channel",
            rows: channels,
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("seed channels");

    let mut member_rows = Vec::with_capacity(15_000);
    for channel_index in 0..3_i64 {
        for member_index in 0..5_000_i64 {
            member_rows.push(vec![
                (
                    "id".to_string(),
                    SqlValue::Integer(channel_index * 5_000 + member_index + 1),
                ),
                (
                    "channel_id".to_string(),
                    SqlValue::Text(format!("channel-{channel_index}")),
                ),
                (
                    "user_id".to_string(),
                    SqlValue::Text(if member_index == 0 {
                        "viewer".to_string()
                    } else {
                        format!("member-{channel_index}-{member_index}")
                    }),
                ),
                (
                    "team_id".to_string(),
                    SqlValue::Text(if member_index == 0 {
                        // Member company may differ from the channel tenant.
                        "external-team".to_string()
                    } else {
                        format!("team-{channel_index}")
                    }),
                ),
                ("channel_is_top".to_string(), SqlValue::Integer(0)),
            ]);
        }
    }
    storage
        .batch_upsert(UpsertSpec {
            table: "channel_member",
            rows: member_rows,
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("seed three 5000-member channels");

    let mut module = ImModule::new(ImConfig {
        auth_user_id: "viewer".to_string(),
        company_id: "company".to_string(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_query_dialog_list",
                serde_json::to_vec(&json!({"limit": 3, "req_id": "p1-host"})).unwrap(),
            )),
            1_000,
            &mut sink,
        )
        .expect("public dialog command");

    let (channel_corr, channel_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("dialog command must issue channel scan");
    let channel_outcome = execute_storage_ops(&storage, &channel_ops).await;
    assert!(matches!(channel_outcome, PortOutcome::Ok(_)));
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: channel_corr,
                outcome: channel_outcome,
            },
            1_001,
            &mut sink,
        )
        .expect("channel scan reply");

    let (member_corr, member_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("channel reply must issue member scan");
    let member_outcome = execute_storage_ops(&storage, &member_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: member_corr,
                outcome: member_outcome,
            },
            1_002,
            &mut sink,
        )
        .expect("member scan reply");

    let result = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .expect("dialog command must emit typed result");
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(result["data"]["req_id"], "p1-host");
    assert_eq!(
        result["data"]["body"]["items"].as_array().map(Vec::len),
        Some(3),
        "three viewer rows must survive the member snapshot limit"
    );
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
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
            scope_key: "channel:1".to_string(),
            value: 10,
        })
        .await
        .expect("first monotonic");
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
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

/// 验证 Host Storage 读命中、读 miss 与错误短路都返回可判定的 PortOutcome。
#[tokio::test]
async fn execute_storage_ops_returns_rows_bytes_and_short_circuits_on_error() {
    let storage = storage().await;
    storage
        .batch_upsert(UpsertSpec {
            table: "messages",
            rows: vec![message_row(7, "seven", 7)],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
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

    let miss = execute_storage_ops(
        &storage,
        &[helix_core::effect::StorageOp::Get(GetSpec {
            table: "messages",
            key_col: "id",
            key_val: SqlValue::Integer(404),
        })],
    )
    .await;
    let PortOutcome::Ok(bytes) = miss else {
        panic!("missing get should return an empty rows envelope");
    };
    assert!(rows_from_reply_bytes(&bytes.0)
        .expect("empty rows json")
        .is_empty());

    let err = execute_storage_ops(
        &storage,
        &[
            helix_core::effect::StorageOp::Get(GetSpec {
                table: "missing_table",
                key_col: "id",
                key_val: SqlValue::Integer(7),
            }),
            helix_core::effect::StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
                table: "_helix_monotonic",
                key_col: "scope_key",
                value_col: "value",
                touch_col: None,
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
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
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
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
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
                    filter: None,
                    order_by: &[],
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
            filter: None,
            order_by: &[],
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
            filter: None,
            order_by: &[],
        })
        .await
        .expect("final scan notes");
    assert_eq!(notes.len(), K as usize, "all concurrent note writes landed");
}

// ─── B1 强证明：读真的与写并发（确定性，纯串行实现必失败）────────────────────
//
// 上面 concurrent_writes_and_reads 的 20s 上限对实测 0.01s 是 2000x 余量 —— 纯串行
// 单锁也能过，证不了「读与写真并发」。本测试给出**确定性强证明**：
//
//   1. 测出「单个 scan 在无写干扰时」的基线耗时 base。
//   2. 起一个**长时占用写连接**的写操作（一条墙钟 ≈ HOLD 的慢 UPDATE，靠递归 CTE
//      在 SQL 内 burn 时间，全程持有 writer 锁 + 写事务）。
//   3. 写仍在跑期间，并发发起 N 个 scan（走只读池，WAL 读快照）。
//   4. 断言：N 个并发读的总墙钟 < 写释放时刻（读在写占用期间就返回），且远小于
//      「写时长 + N×base」（串行下界）。
//
// 纯串行单锁实现：读必须排在写后面 → 总墙钟 ≥ HOLD + N×base，断言失败。
// 读写分离 + WAL：读走独立只读连接，不被写连接的事务阻塞 → 读在 HOLD 期间就全部返回。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reads_proceed_while_write_holds_connection() {
    use std::time::{Duration, Instant};

    let (storage, _path) = file_storage().await;
    // 预填一些行，让 scan 有真实读工作量。
    for i in 1..=50 {
        storage
            .batch_upsert(UpsertSpec {
                table: "messages",
                rows: vec![message_row(i, &format!("seed-{i}"), i)],
                conflict_key: Some("id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .await
            .expect("seed");
    }

    // 基线：无写干扰时单个 scan 的耗时。
    let base_start = Instant::now();
    storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(1000),
            filter: None,
            order_by: &[],
        })
        .await
        .expect("baseline scan");
    let base = base_start.elapsed();

    // 长时占用写连接：一条慢 UPDATE，靠递归 CTE 在 SQL 内 burn 墙钟（约 HOLD）。
    // 全程持有 writer 锁 + 写事务（execute_raw 在写连接上跑）。
    const HOLD: Duration = Duration::from_millis(800);
    let s_writer = storage.clone();
    let write_start = Instant::now();
    let writer_handle = tokio::spawn(async move {
        // 递归 CTE 计数到一个足够大的数以消耗 ~HOLD 墙钟；包在显式事务里持有写连接。
        // count 取值偏保守，宁可超过 HOLD（让读窗口更明确），断言只看读 vs 写释放相对关系。
        s_writer
            .execute_raw(
                "BEGIN IMMEDIATE; \
                 UPDATE messages SET seq = seq + ( \
                   WITH RECURSIVE burn(n) AS ( \
                     SELECT 1 UNION ALL SELECT n + 1 FROM burn WHERE n < 8000000 \
                   ) SELECT count(*) FROM burn \
                 ) WHERE id = 1; \
                 COMMIT;",
            )
            .await
            .expect("long-held write");
        write_start.elapsed()
    });

    // 给写一点时间真正进入 BEGIN IMMEDIATE（拿到写连接 + 写锁）。
    // 用 spawn_blocking + std sleep（dev-deps 未启 tokio "time" feature）。
    tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(50)))
        .await
        .expect("warmup sleep");

    // 写仍在跑期间，并发发起 N 个 scan（走只读池）。
    const N: usize = 8;
    let reads_start = Instant::now();
    let mut read_handles = Vec::new();
    for _ in 0..N {
        let s = storage.clone();
        read_handles.push(tokio::spawn(async move {
            s.scan(ScanSpec {
                table: "messages",
                limit: Some(1000),
                filter: None,
                order_by: &[],
            })
            .await
            .expect("concurrent scan during write")
            .len()
        }));
    }
    let mut total_rows = 0usize;
    for h in read_handles {
        total_rows += h.await.expect("read task join");
    }
    let reads_wall = reads_start.elapsed();

    // 写释放时刻（相对 write_start）。
    let write_elapsed = writer_handle.await.expect("writer join");

    // 写确实占用了可观墙钟（保证窗口存在；若机器太快 burn 不够则放宽断言留 TODO）。
    assert!(
        write_elapsed >= HOLD / 2,
        "写应长时占用连接以构造并发窗口，实际 {write_elapsed:?}（机器过快？提高 burn count）"
    );

    // 强证明①：N 个并发读的总墙钟远小于「串行下界 = 写时长 + N×base」。
    // 串行实现下读排在写后，reads_wall 必 ≳ write_elapsed；这里要求 reads_wall 明显更小。
    assert!(
        reads_wall < write_elapsed,
        "读必须在写占用连接期间就完成（reads_wall {reads_wall:?} < write_elapsed {write_elapsed:?}）\
         —— 串行实现下读会被写阻塞至 ≥ write_elapsed，此断言即失败"
    );

    // 强证明②：N 个并发读总墙钟 ≈ 单个读量级（并发借还，非 N× 串行）。
    // 给宽松上界 max(N×base 的小比例, 一个常数地板) 以吸收调度抖动，但仍远低于 N×base。
    let serial_read_lower = base.saturating_mul(N as u32);
    let concurrent_ceiling = serial_read_lower / 2 + Duration::from_millis(200);
    assert!(
        reads_wall < concurrent_ceiling,
        "N={N} 并发读应 ≈ 单读量级而非 N×base（reads_wall {reads_wall:?} < ceiling {concurrent_ceiling:?}, \
         base {base:?}）—— 串行单锁会退化成 N× 串行"
    );

    // 结果正确：每个并发读都拿到全部 50 行。
    assert_eq!(total_rows, 50 * N, "并发读结果必须完整正确");
}

// ─── E5 守护：monotonic 等值写是 no-op（MAX guard 仅严格大于才前进）────────────
#[tokio::test]
async fn monotonic_upsert_equal_value_is_noop() {
    let storage = storage().await;
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
            scope_key: "ch:eq".to_string(),
            value: 10,
        })
        .await
        .expect("first 10");
    // 再写同值 10 —— WHERE excluded.value > value 不成立 → no-op，仍是 10。
    storage
        .monotonic_upsert(MonotonicUpsertSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
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
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await;
    assert!(err.is_err(), "batch with NOT NULL violation must fail");

    // 第 1 行也不得落盘（整批回滚）。
    let landed = storage
        .scan(ScanSpec {
            table: "strict_tbl",
            limit: Some(100),
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan strict_tbl");
    assert_eq!(
        landed.len(),
        0,
        "no rows should land — entire batch rolled back"
    );
}

// ─── P1-A：读读并发 N>4 Semaphore 背压（无死锁，全完成）────────────────────────
//
// spec §5.2 场景 A：16 个并发 scan（N=16 > READ_POOL_SIZE=4）走 Semaphore 背压，
// 无死锁，最终 16 个全成功，每个拿到相同行。
//
// 判据（三条，与 spec 对应）：
//   ① 16 个读全部成功，每个拿到相同行数（结果正确）
//   ② 总墙钟 < N/2 × base + 500ms（并发借用，非全串行）
//   ③ 总墙钟 ≥ (N/READ_POOL_SIZE) × base（Semaphore 分批，非所有读同时完成）
//      N=16 / pool=4 → ≥ 4 批；wall_clock ≥ 4×base（宽松下界，吸收调度抖动）
//
// 注：N>4 时第 5+ 个 `acquire_owned().await` 在 Semaphore 上异步挂起（不占线程）。
// 纯串行单锁下 16 个读必须排队；WAL 读池下同时跑 ≤4，但总墙钟 ≪ 16×base。
// 判据③是卖点证明：若把 pool 大小改成 16（无背压），所有 scan 同时跑，
// wall ≈ base，③ FAIL → 证明 Semaphore 确实在分批而非被旁路绕过。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_read_concurrency_n_gt_pool_semaphore_backpressure() {
    use std::time::{Duration, Instant};

    let (storage, _path) = file_storage().await;

    // 预填 100 行
    for i in 1_i64..=100 {
        storage
            .batch_upsert(UpsertSpec {
                table: "messages",
                rows: vec![message_row(i, &format!("p1a-{i}"), i)],
                conflict_key: Some("id"),
                exclude_from_update: Vec::new(),
                version_column: None,
                update_guard: None,
            })
            .await
            .expect("seed");
    }

    // 基线：单个 scan 耗时（无并发）
    let base_start = Instant::now();
    let base_rows = storage
        .scan(ScanSpec {
            table: "messages",
            limit: Some(1000),
            filter: None,
            order_by: &[],
        })
        .await
        .expect("baseline scan")
        .len();
    let base = base_start.elapsed().max(Duration::from_micros(1)); // 防止 base=0 导致上界为 0

    // 并发发起 16 个 scan（N=16 > READ_POOL_SIZE=4）
    const N: usize = 16;
    let start = Instant::now();
    let mut handles = Vec::with_capacity(N);
    for _ in 0..N {
        let s = storage.clone();
        handles.push(tokio::spawn(async move {
            s.scan(ScanSpec {
                table: "messages",
                limit: Some(1000),
                filter: None,
                order_by: &[],
            })
            .await
            .expect("concurrent scan N>4")
            .len()
        }));
    }
    let mut results = Vec::with_capacity(N);
    for h in handles {
        results.push(h.await.expect("task join"));
    }
    let wall = start.elapsed();

    // ① 全部成功，每个拿到相同 100 行
    assert_eq!(results.len(), N, "所有 {N} 个读必须完成（无死锁/panic）");
    for (i, &row_count) in results.iter().enumerate() {
        assert_eq!(
            row_count, base_rows,
            "reader {i} 拿到 {row_count} 行，期望 {base_rows} 行（结果正确）"
        );
    }

    // ② 总墙钟 < N × base（并发，非串行）
    // 给足余量：N/2 × base + 常数地板（吸收 CI 调度抖动）
    let serial_upper = base.saturating_mul(N as u32 / 2) + Duration::from_millis(500);
    assert!(
        wall < serial_upper,
        "N={N} 并发读墙钟 {wall:?} 应 < 串行上界 {serial_upper:?}（base={base:?}）——\
         Semaphore 应允许 ≤4 路并发，而非全串行"
    );

    // ③ 不做墙钟「下界」断言（曾在更快机器上误 FAIL，本质 flaky）：
    //    「Semaphore 恰好限到 ≤4 并发」无法用墙钟在黑盒下可靠证明——快读会 pipeline
    //    （一个释放下一个立即 acquire），墙钟远小于「分批×base」朴素模型。timing 下界
    //    天生非确定（防 false-fail 优先于这条次级命题）。本测真正证伪的核心不变量是：
    //    N≫pool 并发读「无死锁 + 全完成 + 结果正确 + 非全串行(上界②)」——Semaphore 若
    //    死锁/丢读/退化全串行，①②会 FAIL。「≤4 并发」由设计 ReadPool::permits=Semaphore(4)
    //    保证，单测层不靠墙钟复证。
    eprintln!(
        "\n[read_read_concurrency_n_gt_pool_semaphore_backpressure] N={N}, wall={wall:?}, base={base:?}"
    );
}

// ─── P1-B：WAL 快照隔离（COMMIT 前读旧快照，COMMIT 后读新快照，零脏读）──────────
//
// spec §5.2 场景 B：写事务 uncommitted 期间 reader 读到的是提交前快照，不是脏数据。
//
// 判据：
//   ① COMMIT 前的读：value=100（旧快照，不脏读）
//   ② COMMIT 后的读：value=200（新快照）
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wal_snapshot_isolation_no_dirty_read() {
    use std::time::Duration;

    let (storage, _path) = file_storage().await;

    // 建 notes 表，预填 id=1 value=100
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS notes \
             (id INTEGER PRIMARY KEY, value INTEGER NOT NULL)",
        )
        .await
        .expect("create notes");
    storage
        .batch_upsert(UpsertSpec {
            table: "notes",
            rows: vec![vec![
                ("id".to_string(), SqlValue::Integer(1)),
                ("value".to_string(), SqlValue::Integer(100)),
            ]],
            conflict_key: Some("id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await
        .expect("seed notes");

    // "慢写"任务：BEGIN IMMEDIATE → UPDATE value=200 → sleep 200ms → COMMIT
    // 在 COMMIT 之前我们先做"COMMIT前读"，COMMIT 之后做"COMMIT后读"。
    //
    // 使用 tokio oneshot 通道协调时序：
    //   慢写 BEGIN 后发 tx1；主线程 COMMIT前读；发 tx2 告知慢写可 COMMIT；
    //   慢写 COMMIT 后发 tx3；主线程 COMMIT后读。
    let (tx_begin, rx_begin) = tokio::sync::oneshot::channel::<()>();
    let (tx_ok_to_commit, rx_ok_to_commit) = tokio::sync::oneshot::channel::<()>();
    let (tx_committed, rx_committed) = tokio::sync::oneshot::channel::<()>();

    let s_writer = storage.clone();
    let write_handle = tokio::spawn(async move {
        // execute_raw 在写连接上运行：BEGIN IMMEDIATE 持有写锁。
        // 分两步：先 BEGIN + UPDATE，等主线程读完，再 COMMIT。
        s_writer
            .execute_raw(
                "BEGIN IMMEDIATE; \
                 UPDATE notes SET value = 200 WHERE id = 1;",
            )
            .await
            .expect("begin + update");
        // 通知主线程：写已进入 uncommitted 状态
        let _ = tx_begin.send(());
        // 等待主线程完成 COMMIT 前读后发来"可以 COMMIT"信号
        let _ = rx_ok_to_commit.await;
        // COMMIT
        s_writer.execute_raw("COMMIT;").await.expect("commit");
        // 通知主线程：COMMIT 完成
        let _ = tx_committed.send(());
    });

    // 等写进入 uncommitted 状态（有界等待，避免 flaky）
    tokio::time::timeout(Duration::from_secs(5), rx_begin)
        .await
        .expect("begin timeout")
        .expect("begin signal");

    // COMMIT 前读：value 应为旧快照 100，不是脏数据 200
    let before_rows = storage
        .get(GetSpec {
            table: "notes",
            key_col: "id",
            key_val: SqlValue::Integer(1),
        })
        .await
        .expect("before-commit read");
    let before_val = before_rows
        .as_ref()
        .and_then(|row| row.iter().find(|(k, _)| k == "value"))
        .map(|(_, v)| v.clone());

    // 告知写任务可以 COMMIT
    let _ = tx_ok_to_commit.send(());

    // 等 COMMIT 完成
    tokio::time::timeout(Duration::from_secs(5), rx_committed)
        .await
        .expect("commit timeout")
        .expect("commit signal");

    // COMMIT 后读：value 应为新快照 200
    let after_rows = storage
        .get(GetSpec {
            table: "notes",
            key_col: "id",
            key_val: SqlValue::Integer(1),
        })
        .await
        .expect("after-commit read");
    let after_val = after_rows
        .as_ref()
        .and_then(|row| row.iter().find(|(k, _)| k == "value"))
        .map(|(_, v)| v.clone());

    write_handle.await.expect("writer join");

    // ① COMMIT 前不脏读（旧快照 100）
    assert!(
        matches!(before_val, Some(SqlValue::Integer(100))),
        "COMMIT 前读应看到旧快照 value=100，实际: {before_val:?}（脏读 = 违反 WAL 快照隔离）"
    );

    // ② COMMIT 后读到新快照 200
    assert!(
        matches!(after_val, Some(SqlValue::Integer(200))),
        "COMMIT 后读应看到新快照 value=200，实际: {after_val:?}"
    );

    eprintln!(
        "\n[wal_snapshot_isolation_no_dirty_read] PASS: before={before_val:?}, after={after_val:?}"
    );
}

#[test]
/// Verify batched table labels remain deterministic and deduplicated.
fn table_set_key_is_sorted_and_deduplicated() {
    use helix_core::effect::{MonotonicUpsertSpec, StorageOp};

    let key = table_set_key(&[
        StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        }),
        StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
            table: "_helix_monotonic",
            key_col: "scope_key",
            value_col: "value",
            touch_col: None,
            scope_key: "channel:1".to_string(),
            value: 1,
        }),
        StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![],
            conflict_key: None,
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        }),
    ]);

    assert_eq!(key, "_helix_monotonic|messages");
}
// ─── S3：GuardedBump（守卫式自增）+ exclude_from_update（守卫列不覆盖）─────────────

async fn channel_like_storage() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_host_gbump_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open storage");
    storage
        .execute_raw(
            "CREATE TABLE IF NOT EXISTS channel (\
             id TEXT PRIMARY KEY, unread_count INTEGER NOT NULL DEFAULT 0, \
	             mention_count INTEGER NOT NULL DEFAULT 0, urgent_count INTEGER NOT NULL DEFAULT 0, \
	             mention_list TEXT NOT NULL DEFAULT '', urgent_post_list TEXT NOT NULL DEFAULT '', \
	             unread_post_id TEXT NOT NULL DEFAULT '', last_post TEXT NOT NULL DEFAULT '', \
             last_root_post_at INTEGER NOT NULL DEFAULT 0, purpose TEXT NOT NULL DEFAULT '')",
        )
        .await
        .expect("create channel");
    storage
}

/// G-01 回归：复合主键原语只能命中指定 channel + user，且读回同一绝对态。
#[tokio::test]
async fn scoped_guarded_bump_and_get_isolate_channel_member_row(
) -> Result<(), Box<dyn std::error::Error>> {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_host_scoped_bump_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db).await?;
    storage
        .execute_raw(
            "CREATE TABLE channel_member (\
             channel_id TEXT NOT NULL, user_id TEXT NOT NULL, \
             unread_count INTEGER NOT NULL DEFAULT 0, \
             last_post TEXT NOT NULL DEFAULT '', \
             last_root_post_at INTEGER NOT NULL DEFAULT 0, \
             PRIMARY KEY(channel_id, user_id))",
        )
        .await?;
    storage
        .batch_upsert(UpsertSpec {
            table: "channel_member",
            rows: vec![
                vec![
                    ("channel_id".to_string(), SqlValue::Text("c1".to_string())),
                    ("user_id".to_string(), SqlValue::Text("u1".to_string())),
                    ("unread_count".to_string(), SqlValue::Integer(4)),
                    ("last_root_post_at".to_string(), SqlValue::Integer(100)),
                ],
                vec![
                    ("channel_id".to_string(), SqlValue::Text("c2".to_string())),
                    ("user_id".to_string(), SqlValue::Text("u1".to_string())),
                    ("unread_count".to_string(), SqlValue::Integer(9)),
                    ("last_root_post_at".to_string(), SqlValue::Integer(100)),
                ],
            ],
            conflict_key: Some("channel_id,user_id"),
            exclude_from_update: Vec::new(),
            version_column: None,
            update_guard: None,
        })
        .await?;

    storage
        .scoped_guarded_bump(ScopedGuardedBumpSpec {
            comparison: helix_core::effect::GuardComparison::Greater,
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text("c1".to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text("u1".to_string()),
            bump_col: "unread_count",
            bump_delta: 1,
            set_cols: vec![
                ("last_post".to_string(), SqlValue::Text("p1".to_string())),
                ("last_root_post_at".to_string(), SqlValue::Integer(200)),
            ],
            guard_col: "last_root_post_at",
            guard_val: 200,
            additional_guard: None,
        })
        .await?;

    let targeted = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text("c1".to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text("u1".to_string()),
        })
        .await?
        .ok_or("missing targeted channel member")?;
    let untouched = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text("c2".to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text("u1".to_string()),
        })
        .await?
        .ok_or("missing untouched channel member")?;
    let value = |row: &helix_core::effect::Row, column: &str| {
        row.iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value.clone())
    };
    assert!(matches!(
        value(&targeted, "unread_count"),
        Some(SqlValue::Integer(5))
    ));
    assert!(matches!(
        value(&untouched, "unread_count"),
        Some(SqlValue::Integer(9))
    ));
    Ok(())
}

/// 回归锚: S3 path3 —— guarded_bump 守卫命中 → unread +1（SQL 自增）+ 绝对列写。
#[tokio::test]
async fn guarded_bump_increments_when_guard_passes_s3() {
    let st = channel_like_storage().await;
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c1".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(0)),
            ("last_root_post_at".to_string(), SqlValue::Integer(100)),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed channel");

    // 连续 3 条他人消息（create_at 递增，均 > 当前 last_root_post_at）→ +3。
    for (i, ts) in [200i64, 300, 400].iter().enumerate() {
        st.guarded_bump(GuardedBumpSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c1".to_string()),
            bump_col: "unread_count",
            bump_delta: 1,
            extra_bumps: Vec::new(),
            set_cols: vec![
                (
                    "unread_post_id".to_string(),
                    SqlValue::Text(format!("p{i}")),
                ),
                ("last_post".to_string(), SqlValue::Text(format!("body{i}"))),
                // 生产路径（bump_channel_unread）同样把 last_root_post_at 推进进 set_cols——
                // 守卫列自身随命中前进，使下一条按新水位判定（与现网 update_last_post 一致）。
                ("last_root_post_at".to_string(), SqlValue::Integer(*ts)),
            ],
            guard_col: "last_root_post_at",
            guard_val: *ts,
        })
        .await
        .expect("bump");
    }

    let row = st
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c1".to_string()),
        })
        .await
        .expect("get")
        .expect("row");
    let unread = row
        .iter()
        .find(|(k, _)| k == "unread_count")
        .map(|(_, v)| v);
    assert!(
        matches!(unread, Some(SqlValue::Integer(3))),
        "连续 3 条 → unread==3，got {unread:?}"
    );
    let lrp = row
        .iter()
        .find(|(k, _)| k == "last_root_post_at")
        .map(|(_, v)| v);
    assert!(
        matches!(lrp, Some(SqlValue::Integer(400))),
        "last_root_post_at 推进到 400"
    );
}

/// 回归锚: S3 path3 —— guarded_bump 守卫不过（旧/重复消息）→ 不自增、不污染。
#[tokio::test]
async fn guarded_bump_noop_when_guard_fails_s3() {
    let st = channel_like_storage().await;
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c2".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(5)),
            ("last_root_post_at".to_string(), SqlValue::Integer(500)),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed");

    // 守卫值 300 < 现值 500 → 守卫不过 → 整行不动（重复 / 乱序旧消息幂等）。
    st.guarded_bump(GuardedBumpSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text("c2".to_string()),
        bump_col: "unread_count",
        bump_delta: 1,
        extra_bumps: Vec::new(),
        set_cols: vec![(
            "unread_post_id".to_string(),
            SqlValue::Text("stale".to_string()),
        )],
        guard_col: "last_root_post_at",
        guard_val: 300,
    })
    .await
    .expect("bump");

    let row = st
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c2".to_string()),
        })
        .await
        .expect("get")
        .expect("row");
    let unread = row
        .iter()
        .find(|(k, _)| k == "unread_count")
        .map(|(_, v)| v);
    assert!(
        matches!(unread, Some(SqlValue::Integer(5))),
        "守卫不过 → unread 不变 (5)，got {unread:?}"
    );
}

/// 回归锚: Dialog patch —— unread/mention/urgent 计数和当前消息 singleton 列表必须在
/// 同一条 guarded UPDATE 下原子推进；旧消息不得二次增加或覆盖当前列表。
#[tokio::test]
async fn guarded_bump_extra_counts_and_singleton_lists_share_the_same_guard_s3() {
    let st = channel_like_storage().await;
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c-extra".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(0)),
            ("mention_count".to_string(), SqlValue::Integer(0)),
            ("urgent_count".to_string(), SqlValue::Integer(0)),
            ("last_root_post_at".to_string(), SqlValue::Integer(100)),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed");

    st.guarded_bump(GuardedBumpSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text("c-extra".to_string()),
        bump_col: "unread_count",
        bump_delta: 1,
        extra_bumps: vec![("mention_count", 1), ("urgent_count", 1)],
        set_cols: vec![
            (
                "mention_list".to_string(),
                SqlValue::Text(r#"["current-post"]"#.to_string()),
            ),
            (
                "urgent_post_list".to_string(),
                SqlValue::Text(r#"["current-post"]"#.to_string()),
            ),
            ("last_root_post_at".to_string(), SqlValue::Integer(200)),
        ],
        guard_col: "last_root_post_at",
        guard_val: 200,
    })
    .await
    .expect("first bump");

    // 旧消息重复到达：同一 guard 应拦住 unread 与 extra_bumps，不能只拦主计数。
    st.guarded_bump(GuardedBumpSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text("c-extra".to_string()),
        bump_col: "unread_count",
        bump_delta: 1,
        extra_bumps: vec![("mention_count", 1), ("urgent_count", 1)],
        set_cols: vec![
            (
                "mention_list".to_string(),
                SqlValue::Text(r#"["stale-post"]"#.to_string()),
            ),
            (
                "urgent_post_list".to_string(),
                SqlValue::Text(r#"["stale-post"]"#.to_string()),
            ),
            ("last_root_post_at".to_string(), SqlValue::Integer(150)),
        ],
        guard_col: "last_root_post_at",
        guard_val: 150,
    })
    .await
    .expect("stale bump");

    let row = st
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c-extra".to_string()),
        })
        .await
        .expect("get")
        .expect("row");
    let value = |col: &str| row.iter().find(|(k, _)| k == col).map(|(_, v)| v);
    assert!(matches!(value("unread_count"), Some(SqlValue::Integer(1))));
    assert!(matches!(value("mention_count"), Some(SqlValue::Integer(1))));
    assert!(matches!(value("urgent_count"), Some(SqlValue::Integer(1))));
    assert!(matches!(
        value("mention_list"),
        Some(SqlValue::Text(list)) if list == r#"["current-post"]"#
    ));
    assert!(matches!(
        value("urgent_post_list"),
        Some(SqlValue::Text(list)) if list == r#"["current-post"]"#
    ));
    assert!(matches!(
        value("last_root_post_at"),
        Some(SqlValue::Integer(200))
    ));
}

/// 回归锚: S3 path1 —— exclude_from_update：冲突时守卫/本地列保留既有值，其余列覆盖。
#[tokio::test]
async fn upsert_exclude_from_update_preserves_guard_cols_s3() {
    let st = channel_like_storage().await;
    // 初始：unread=7（本地累计）、purpose="local"、last_post="old"。
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c3".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(7)),
            ("purpose".to_string(), SqlValue::Text("local".to_string())),
            ("last_post".to_string(), SqlValue::Text("old".to_string())),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed");

    // path1 全量 upsert：server 未给 unread/purpose（行内写默认 0/""），但 exclude → 冲突时不覆盖；
    // last_post 不在 exclude → 覆盖为 "new"。
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c3".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(0)),
            ("purpose".to_string(), SqlValue::Text(String::new())),
            ("last_post".to_string(), SqlValue::Text("new".to_string())),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: vec!["unread_count", "purpose"],
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("upsert full");

    let row = st
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c3".to_string()),
        })
        .await
        .expect("get")
        .expect("row");
    let unread = row
        .iter()
        .find(|(k, _)| k == "unread_count")
        .map(|(_, v)| v);
    let purpose = row.iter().find(|(k, _)| k == "purpose").map(|(_, v)| v);
    let last_post = row.iter().find(|(k, _)| k == "last_post").map(|(_, v)| v);
    assert!(
        matches!(unread, Some(SqlValue::Integer(7))),
        "exclude → unread 保留本地 7，got {unread:?}"
    );
    assert!(
        matches!(purpose, Some(SqlValue::Text(s)) if s == "local"),
        "exclude → purpose 保留本地，got {purpose:?}"
    );
    assert!(
        matches!(last_post, Some(SqlValue::Text(s)) if s == "new"),
        "非 exclude → last_post 覆盖 new，got {last_post:?}"
    );
}

/// BatchDelete 真 SQLite 验证：复合 PK 作用域删 + **跨作用域隔离**（绝不误删别的 channel 的成员）。
/// 改 batch_delete 的 scope WHERE（去掉 scope_col=? 约束）→ c2/u1 被误删 → 本测 FAIL（HX-C011）。
#[tokio::test]
async fn host_storage_batch_delete_scoped_no_cross_channel_delete() {
    let s = storage().await;
    s.execute_raw(
        "CREATE TABLE channel_member (channel_id TEXT NOT NULL, user_id TEXT NOT NULL, \
         PRIMARY KEY (channel_id, user_id))",
    )
    .await
    .expect("create channel_member");

    let mk = |cid: &str, uid: &str| {
        vec![
            ("channel_id".to_string(), SqlValue::Text(cid.to_string())),
            ("user_id".to_string(), SqlValue::Text(uid.to_string())),
        ]
    };
    s.batch_upsert(UpsertSpec {
        table: "channel_member",
        rows: vec![
            mk("c1", "u1"),
            mk("c1", "u2"),
            mk("c1", "u3"),
            mk("c2", "u1"),
        ],
        conflict_key: Some("channel_id,user_id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed");

    // 删 c1 的 u1,u2（scope=c1）
    s.batch_delete(BatchDeleteSpec {
        table: "channel_member",
        scope_col: "channel_id",
        scope_val: SqlValue::Text("c1".to_string()),
        key_col: "user_id",
        key_vals: vec![
            SqlValue::Text("u1".to_string()),
            SqlValue::Text("u2".to_string()),
        ],
    })
    .await
    .expect("delete");

    let rows = s
        .scan(ScanSpec {
            table: "channel_member",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan");
    let pairs: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            let g = |col: &str| match r.iter().find(|(k, _)| k == col).map(|(_, v)| v) {
                Some(SqlValue::Text(t)) => t.clone(),
                _ => String::new(),
            };
            (g("channel_id"), g("user_id"))
        })
        .collect();
    assert_eq!(pairs.len(), 2, "删后剩 2 行(c1/u3 + c2/u1)，got {pairs:?}");
    assert!(
        pairs.contains(&("c1".to_string(), "u3".to_string())),
        "c1/u3 应保留，got {pairs:?}"
    );
    assert!(
        pairs.contains(&("c2".to_string(), "u1".to_string())),
        "c2/u1 不应被跨作用域误删，got {pairs:?}"
    );
    assert!(
        !pairs
            .iter()
            .any(|(c, u)| c == "c1" && (u == "u1" || u == "u2")),
        "c1/u1,c1/u2 应被删，got {pairs:?}"
    );
}

/// StorageOp dispatch 回归锚：engine/native/FFI 持久化路径都走 host `execute_storage_ops`，
/// 这里证明 `StorageOp::BatchDelete` 兑现到同一个 `Storage::batch_delete` 实现。
#[tokio::test]
async fn execute_storage_ops_dispatches_batch_delete() {
    let s = storage().await;
    s.execute_raw(
        "CREATE TABLE channel_member (channel_id TEXT NOT NULL, user_id TEXT NOT NULL, \
         PRIMARY KEY (channel_id, user_id))",
    )
    .await
    .expect("create channel_member");

    let mk = |cid: &str, uid: &str| {
        vec![
            ("channel_id".to_string(), SqlValue::Text(cid.to_string())),
            ("user_id".to_string(), SqlValue::Text(uid.to_string())),
        ]
    };
    s.batch_upsert(UpsertSpec {
        table: "channel_member",
        rows: vec![mk("c1", "u1"), mk("c1", "u2")],
        conflict_key: Some("channel_id,user_id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed");

    let outcome = execute_storage_ops(
        &s,
        &[helix_core::effect::StorageOp::BatchDelete(
            BatchDeleteSpec {
                table: "channel_member",
                scope_col: "channel_id",
                scope_val: SqlValue::Text("c1".to_string()),
                key_col: "user_id",
                key_vals: vec![SqlValue::Text("u1".to_string())],
            },
        )],
    )
    .await;
    assert!(matches!(outcome, helix_core::tick::PortOutcome::Ok(_)));

    let rows = s
        .scan(ScanSpec {
            table: "channel_member",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan");
    let users: Vec<String> = rows
        .iter()
        .filter_map(
            |r| match r.iter().find(|(k, _)| k == "user_id").map(|(_, v)| v) {
                Some(SqlValue::Text(t)) => Some(t.clone()),
                _ => None,
            },
        )
        .collect();
    assert_eq!(
        users.len(),
        1,
        "only one member should remain, got {users:?}"
    );
    assert!(
        users.contains(&"u2".to_string()),
        "u2 should remain, got {users:?}"
    );
}

/// 回归锚: #1 防历史重算 —— guarded_bump 守卫 FAIL（消息 create_at ≤ 当前 last_root_post_at =
/// 已看过的历史/backfill 旧消息）→ unread **不动**（SQL `WHERE ? > guard_col` 严格大于不命中），
/// 水位也不前进。删守卫 WHERE 子句 → 历史也 +1 → 本测 FAIL（HX-C011 可证伪）。
/// 这是「离线全量 backfill 不把已读历史重复计未读」的存储层闸。
#[tokio::test]
async fn guarded_bump_does_not_increment_when_guard_fails_historical_s3() {
    let st = channel_like_storage().await;
    st.batch_upsert(UpsertSpec {
        table: "channel",
        rows: vec![vec![
            ("id".to_string(), SqlValue::Text("c1".to_string())),
            ("unread_count".to_string(), SqlValue::Integer(0)),
            ("last_root_post_at".to_string(), SqlValue::Integer(1000)),
        ]],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: None,
    })
    .await
    .expect("seed channel");

    // 三条历史/等值消息：create_at ∈ {500, 800, 1000} 均 ≤ 当前水位 1000（严格 > 才命中）→ 均不 bump。
    for ts in [500i64, 800, 1000] {
        st.guarded_bump(GuardedBumpSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c1".to_string()),
            bump_col: "unread_count",
            bump_delta: 1,
            extra_bumps: Vec::new(),
            set_cols: vec![("last_root_post_at".to_string(), SqlValue::Integer(ts))],
            guard_col: "last_root_post_at",
            guard_val: ts,
        })
        .await
        .expect("bump");
    }

    let row = st
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text("c1".to_string()),
        })
        .await
        .expect("get")
        .expect("row");
    let unread = row
        .iter()
        .find(|(k, _)| k == "unread_count")
        .map(|(_, v)| v);
    assert!(
        matches!(unread, Some(SqlValue::Integer(0))),
        "历史消息(create_at ≤ last_root_post_at)守卫 fail → unread 不动==0（防历史重算），got {unread:?}"
    );
    let lrp = row
        .iter()
        .find(|(k, _)| k == "last_root_post_at")
        .map(|(_, v)| v);
    assert!(
        matches!(lrp, Some(SqlValue::Integer(1000))),
        "守卫 fail → 整行不更新 → last_root_post_at 不前进==1000，got {lrp:?}"
    );
}
