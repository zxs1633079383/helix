//! SQLite 回归：守卫式 upsert 失败时必须回滚整个 PersistAtomic 批次。

use helix_core::effect::{GetSpec, Row, SqlValue, StorageOp, UpsertGuard, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::PortError;
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Open an isolated in-memory database owned by this test.
async fn storage() -> HostStorage {
    let sequence = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_upsert_guard_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        sequence
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open isolated guard storage");
    storage
        .execute_raw(
            "CREATE TABLE guarded_fact (
                id TEXT PRIMARY KEY,
                props TEXT,
                fact TEXT
            )",
        )
        .await
        .expect("create guarded_fact");
    storage
}

/// Construct the text-only fixture row.
fn row(id: &str, props: &str, fact: &str) -> Row {
    vec![
        ("id".to_string(), SqlValue::Text(id.to_string())),
        ("props".to_string(), SqlValue::Text(props.to_string())),
        ("fact".to_string(), SqlValue::Text(fact.to_string())),
    ]
}

/// Build a compare-and-swap update against the original props.
fn guarded_upsert(id: &str, props: &str, fact: &str, expected_props: &str) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec {
        table: "guarded_fact",
        rows: vec![row(id, props, fact)],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: Some(UpsertGuard {
            column: "props",
            expected: SqlValue::Text(expected_props.to_string()),
        }),
    })
}

/// Build the preceding write whose rollback is observable.
fn unguarded_upsert(id: &str, props: &str, fact: &str) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec::new(
        "guarded_fact",
        vec![row(id, props, fact)],
        Some("id"),
    ))
}

/// Read persisted columns through the real Storage port.
async fn get_row(storage: &HostStorage, id: &str) -> Option<Row> {
    storage
        .get(GetSpec {
            table: "guarded_fact",
            key_col: "id",
            key_val: SqlValue::Text(id.to_string()),
        })
        .await
        .expect("read guarded_fact")
}

/// 旧 props 不匹配时，先写入的事实和守卫行都必须留在事务前状态。
#[tokio::test]
async fn guarded_upsert_mismatch_rolls_back_atomic_facts() {
    let storage = storage().await;
    storage
        .batch_upsert(UpsertSpec::new(
            "guarded_fact",
            vec![row("existing", "old", "stable")],
            Some("id"),
        ))
        .await
        .expect("seed guarded row");

    let ops = vec![
        unguarded_upsert("new-fact", "new", "must-rollback"),
        guarded_upsert("existing", "new", "must-not-apply", "stale"),
    ];
    let outcome = execute_storage_ops_atomic(&storage, &ops).await;
    assert!(matches!(outcome, helix_core::tick::PortOutcome::Err(_)));

    let direct_error = storage
        .atomic_write(ops)
        .await
        .expect_err("guard must fail");
    assert!(matches!(direct_error, PortError::StorageConflict));
    assert!(get_row(&storage, "new-fact").await.is_none());
    assert_row(
        get_row(&storage, "existing").await,
        row("existing", "old", "stable"),
    );
}

/// 匹配旧 props 时更新；缺行且 expected=NULL 时仍走正常 INSERT。
#[tokio::test]
async fn guarded_upsert_matches_and_inserts_missing_null_version() {
    let storage = storage().await;
    storage
        .batch_upsert(UpsertSpec::new(
            "guarded_fact",
            vec![row("existing", "old", "stable")],
            Some("id"),
        ))
        .await
        .expect("seed guarded row");

    let update = execute_storage_ops(
        &storage,
        &[guarded_upsert("existing", "new", "updated", "old")],
    )
    .await;
    assert!(matches!(update, helix_core::tick::PortOutcome::Ok(_)));
    assert_row(
        get_row(&storage, "existing").await,
        row("existing", "new", "updated"),
    );

    let insert = StorageOp::BatchUpsert(UpsertSpec {
        table: "guarded_fact",
        rows: vec![row("missing", "new", "inserted")],
        conflict_key: Some("id"),
        exclude_from_update: Vec::new(),
        version_column: None,
        update_guard: Some(UpsertGuard {
            column: "props",
            expected: SqlValue::Null,
        }),
    });
    let outcome = execute_storage_ops_atomic(&storage, &[insert]).await;
    assert!(matches!(outcome, helix_core::tick::PortOutcome::Ok(_)));
    assert_row(
        get_row(&storage, "missing").await,
        row("missing", "new", "inserted"),
    );
}

/// Compare actual text columns by key, independent of SQLite column order.
fn assert_row(actual: Option<Row>, expected: Row) {
    assert_eq!(
        text_columns(actual.expect("persisted row")),
        text_columns(expected)
    );
}

/// Reject unexpected storage types instead of comparing debug formatting.
fn text_columns(row: Row) -> std::collections::BTreeMap<String, String> {
    row.into_iter()
        .map(|(key, value)| {
            let SqlValue::Text(value) = value else {
                panic!("expected text column {key}")
            };
            (key, value)
        })
        .collect()
}

/// Version guards skip old snapshots, admit equal/new values and retain exact u64 ordering.
#[tokio::test]
async fn versioned_upsert_keeps_monotonic_facts_and_default_updates() {
    let storage = storage().await;
    for (version, expected) in [
        (10u64, 10),
        (9, 10),
        (10, 10),
        (9007199254740993, 9007199254740993),
        (u64::MAX, u64::MAX),
        (u64::MAX - 1, u64::MAX),
    ] {
        let mut spec = UpsertSpec::new(
            "guarded_fact",
            vec![row("versioned", &format!("{version:020}"), "content")],
            Some("id"),
        );
        spec.version_column = Some("props");
        storage.batch_upsert(spec).await.unwrap();
        let saved = get_row(&storage, "versioned").await.unwrap();
        assert!(saved.iter().any(|(key, value)| key == "props"
            && matches!(value, SqlValue::Text(text) if text == &format!("{expected:020}"))));
    }
    let mut stale = UpsertSpec::new(
        "guarded_fact",
        vec![row("versioned", "00000000000000000001", "old")],
        Some("id"),
    );
    stale.version_column = Some("props");
    stale.update_guard = Some(UpsertGuard {
        column: "props",
        expected: SqlValue::Text(format!("{:020}", u64::MAX)),
    });
    let outcome = execute_storage_ops_atomic(
        &storage,
        &[
            unguarded_upsert("rollback", "x", "x"),
            StorageOp::BatchUpsert(stale),
        ],
    )
    .await;
    assert!(matches!(outcome, helix_core::tick::PortOutcome::Err(_)));
    assert!(get_row(&storage, "rollback").await.is_none());
    // Default specs still allow ordinary replacements, including legacy text-chain writes.
    storage
        .batch_upsert(UpsertSpec::new(
            "guarded_fact",
            vec![row("versioned", "old", "default")],
            Some("id"),
        ))
        .await
        .unwrap();
    assert!(get_row(&storage, "versioned")
        .await
        .unwrap()
        .iter()
        .any(
            |(key, value)| key == "props" && matches!(value, SqlValue::Text(text) if text == "old")
        ));
}

/// 迟到话题清单即使来自另一写队列，也必须整批回滚，保留新数量和话题行。
#[tokio::test]
async fn stale_topic_snapshot_cannot_replace_newer_collection() {
    use helix_core::effect::BatchUpdateSpec;
    let storage = storage().await;
    storage.execute_raw("CREATE TABLE channel (id TEXT PRIMARY KEY, root_id TEXT NOT NULL DEFAULT '', is_remove INTEGER NOT NULL DEFAULT 0, subtopics_loaded_at INTEGER NOT NULL DEFAULT 0, thread_count INTEGER NOT NULL DEFAULT 0); INSERT INTO channel(id,subtopics_loaded_at,thread_count) VALUES ('parent',200,4); INSERT INTO channel(id,root_id) VALUES ('new-topic','parent');").await.unwrap();
    let ops = vec![
        StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "root_id",
            key_vals: vec![SqlValue::Text("parent".into())],
            patch: vec![("is_remove".into(), SqlValue::Integer(1))],
        }),
        StorageOp::BatchUpsert(UpsertSpec {
            table: "channel",
            rows: vec![vec![
                ("id".into(), SqlValue::Text("parent".into())),
                ("subtopics_loaded_at".into(), SqlValue::Integer(150)),
                ("thread_count".into(), SqlValue::Integer(0)),
            ]],
            conflict_key: Some("id"),
            exclude_from_update: vec![],
            version_column: None,
            update_guard: Some(UpsertGuard {
                column: "subtopics_loaded_at",
                expected: SqlValue::Integer(100),
            }),
        }),
    ];
    assert!(matches!(
        storage.atomic_write(ops).await,
        Err(PortError::StorageConflict)
    ));
    for (id, column, expected) in [("parent", "thread_count", 4), ("new-topic", "is_remove", 0)] {
        let row = storage
            .get(GetSpec {
                table: "channel",
                key_col: "id",
                key_val: SqlValue::Text(id.into()),
            })
            .await
            .unwrap()
            .unwrap();
        assert!(row
            .iter()
            .any(|(key, value)| key == column
                && matches!(value,SqlValue::Integer(n) if *n==expected)));
    }
}

/// 真实 Module + SQLite：快照写在途时加入新话题，重启后迟到旧全集不能回退落库集合。
#[tokio::test]
async fn topic_inventory_lifecycle_race_survives_restart() {
    use helix_core::tick::InboundBytes;
    use helix_core::{Effect, EffectSink, Module, Tick};
    use helix_im::{ImConfig, ImModule};
    use serde_json::json;
    const P: &str = "chfixx00000000000000000064";
    const T: &str = "chfixx00000000000000000065";
    let db = storage().await;
    for sql in helix_im::schema::IM_SCHEMA {
        db.execute_raw(sql).await.unwrap();
    }
    db.execute_raw("INSERT INTO channel(id,type,user_id) VALUES('chfixx00000000000000000064','P','viewer'),('chfixx00000000000000000065','T','viewer')")
    .await
    .unwrap();
    db.execute_raw("UPDATE channel SET root_id='chfixx00000000000000000064' WHERE id='chfixx00000000000000000065'")
        .await
        .unwrap();
    let config = || ImConfig {
        auth_user_id: "viewer".into(),
        ..ImConfig::default()
    };
    let mut module = ImModule::new(config());
    let mut sink = EffectSink::new();
    let old = json!({"action":"increment_channel_end","data":{"channelId":P,"scope":"subtopics","batchId":"old","requestId":"old","snapshotComplete":true,"channelCount":0,"topicInventory":{"version":"201","channelIds":[]}}});
    module
        .handle(
            &Tick::Inbound(InboundBytes(old.to_string().into())),
            100,
            &mut sink,
        )
        .unwrap();
    let Effect::Persist { corr, ops } = &sink.as_slice()[0] else {
        panic!("read")
    };
    let corr = *corr;
    let result = execute_storage_ops(&db, ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: result,
            },
            100,
            &mut sink,
        )
        .unwrap();
    let Effect::PersistAtomic {
        corr: old_corr,
        ops: old_ops,
    } = &sink.as_slice()[0]
    else {
        panic!("atomic")
    };
    let old_corr = *old_corr;
    let old_ops = old_ops.clone();
    sink.clear();
    let new = json!({"action":"update_channel","data":{"id":P,"userId":"viewer","threadCount":1,"topicInventory":{"version":"202","channelIds":[T]}}});
    module
        .handle(
            &Tick::Inbound(InboundBytes(new.to_string().into())),
            101,
            &mut sink,
        )
        .unwrap();
    assert!(sink.as_slice().is_empty());
    let result = execute_storage_ops_atomic(&db, &old_ops).await;
    module
        .handle(
            &Tick::PortReply {
                corr: old_corr,
                outcome: result,
            },
            101,
            &mut sink,
        )
        .unwrap();
    // 真实存储端口驱动剩余队列，不能用伪造成功代替提交。
    for _ in 0..6 {
        let next = sink.as_slice().iter().find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone(), false)),
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone(), true)),
            _ => None,
        });
        let Some((corr, ops, atomic)) = next else {
            break;
        };
        sink.clear();
        let outcome = if atomic {
            execute_storage_ops_atomic(&db, &ops).await
        } else {
            execute_storage_ops(&db, &ops).await
        };
        module
            .handle(&Tick::PortReply { corr, outcome }, 102, &mut sink)
            .unwrap();
    }
    let parent = db
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(P.into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(parent
        .iter()
        .any(|(k, v)| k == "topic_inventory_version" && matches!(v, SqlValue::Integer(202))));
    assert!(parent
        .iter()
        .any(|(k, v)| k == "thread_count" && matches!(v, SqlValue::Integer(1))));
    assert!(parent.iter().any(|(k, v)| k == "topic_inventory_ids"
        && matches!(v,SqlValue::Text(ids) if ids==&json!([T]).to_string())));
    let topic = db
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(T.into()),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(topic
        .iter()
        .any(|(k, v)| k == "is_remove" && matches!(v, SqlValue::Integer(0))));
    let mut restarted = ImModule::new(config());
    sink.clear();
    restarted
        .handle(
            &Tick::Inbound(InboundBytes(old.to_string().into())),
            103,
            &mut sink,
        )
        .unwrap();
    let Effect::Persist { corr, ops } = &sink.as_slice()[0] else {
        panic!("restart read")
    };
    let corr = *corr;
    let outcome = execute_storage_ops(&db, ops).await;
    sink.clear();
    restarted
        .handle(&Tick::PortReply { corr, outcome }, 103, &mut sink)
        .unwrap();
    assert!(
        sink.as_slice().is_empty(),
        "durably stale observation must not persist or emit"
    );
}
