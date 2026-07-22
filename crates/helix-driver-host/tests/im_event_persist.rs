//! 真 DB 落库集成测试（防"绿测试锁死 bug"陷阱）。
//!
//! ## 为什么需要这个测试
//!
//! 此前 helix-im 的事件落库测试只断言 `Effect` 里 `StorageOp.table` 字符串，
//! 恰好把错误表名（不存在的 `messages`）固化为期望值——绿测试反而锁死了 bug：
//! 两条 live 写路径（WS 推送 / B-rest 增量）都写向永不存在的表，运行时必 `no such
//! table: messages`，sync cursor 永不推进 → 无限重拉风暴。
//!
//! 本测试用真 `IM_SCHEMA` 建真 SQLite（rusqlite，host driver 同款执行器），
//! 喂 `event_to_upsert_op` / `batch_upsert_events` 产物，**断言 `message` 表真有行**
//! （而非断 Effect 字符串）。表名错配 / 缺列 / PK 撞键任一回归都会让此测试红。

use helix_core::effect::{BatchUpdateSpec, GetSpec, ScanSpec, SqlValue, StorageOp, UpsertSpec};
use helix_core::ports::Storage;
use helix_driver_host::{execute_storage_ops_atomic, HostStorage};
use helix_im::acl::to_effect::terminal_tombstone_and_cursor_op;
use helix_im::channel::event_to_upsert_op;
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::{EventEnvelope, EventKind, PostFields};
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

fn cid(n: u64) -> String {
    format!("chfixx{n:020x}")
}

fn channel_id(n: u64) -> ChannelId {
    ChannelId::from_str(&cid(n)).expect("test channel id fixture must be valid Id26")
}

fn is_duplicate_column_error(error: &impl std::fmt::Display) -> bool {
    error.to_string().contains("duplicate column name")
}

async fn apply_im_schema(storage: &HostStorage) {
    for &sql in helix_im::IM_SCHEMA {
        storage
            .execute_raw(sql)
            .await
            .unwrap_or_else(|e| panic!("execute_raw IM_SCHEMA failed for {sql:?}: {e:?}"));
    }
    for &sql in helix_im::IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                is_duplicate_column_error(&error),
                "execute_raw IM_SCHEMA_MIGRATIONS failed for {sql:?}: {error:?}"
            );
        }
    }
}

/// 以真 `IM_SCHEMA` + additive migrations 建库（含 message / channel / channel_member；
/// cursor 真源是 _helix_monotonic）。
async fn storage_with_im_schema() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_im_persist_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open storage");
    // host-cli / FFI 启动同款：建表后执行 additive migrations，并忽略已存在列。
    apply_im_schema(&storage).await;
    storage
}

/// 旧库的 `CREATE TABLE IF NOT EXISTS` 不会补新增列；启动 migration 必须把
/// `reply_count` 加回，随后同一 WS upsert 才能成功。
#[tokio::test]
async fn existing_message_table_adds_reply_count_before_event_persist() {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_im_legacy_persist_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open legacy storage");
    storage
        .execute_raw(
            "CREATE TABLE message (\
                temporary_id TEXT PRIMARY KEY,\
                id TEXT NOT NULL DEFAULT '',\
                channel_id TEXT NOT NULL DEFAULT '',\
                user_id TEXT NOT NULL DEFAULT '',\
                type TEXT NOT NULL DEFAULT '',\
                message TEXT NOT NULL DEFAULT '',\
                props TEXT NOT NULL DEFAULT '',\
                viewers TEXT NOT NULL DEFAULT '',\
                user_snapshot TEXT NOT NULL DEFAULT '',\
                mentions TEXT NOT NULL DEFAULT '',\
                replied_message TEXT NOT NULL DEFAULT '',\
                reply_messages TEXT NOT NULL DEFAULT '',\
                reply_id TEXT NOT NULL DEFAULT '',\
                reply_root_id TEXT NOT NULL DEFAULT '',\
                reply_first_level_id TEXT NOT NULL DEFAULT '',\
                expedite_map TEXT NOT NULL DEFAULT '',\
                topic TEXT NOT NULL DEFAULT '',\
                quick_reply TEXT NOT NULL DEFAULT '',\
                simple_message TEXT NOT NULL DEFAULT '',\
                read_bits TEXT NOT NULL DEFAULT '',\
                send_status TEXT NOT NULL DEFAULT '',\
                revoke INTEGER NOT NULL DEFAULT 0,\
                create_at INTEGER NOT NULL DEFAULT 0,\
                update_at INTEGER NOT NULL DEFAULT 0\
            )",
        )
        .await
        .expect("create legacy message table without reply_count");

    apply_im_schema(&storage).await;
    let mut event = post_event(42, 6, "tmp-legacy", "post-legacy", "legacy migrated");
    event.fields.reply_count = 3;
    let op = event_to_upsert_op(&event);
    storage
        .batch_upsert(match op {
            helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("migration must add reply_count before event persist");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 1);
    match col(&rows[0], "reply_count") {
        Some(SqlValue::Integer(value)) => assert_eq!(*value, 3),
        other => panic!("reply_count expected Integer(3), got {other:?}"),
    }
}

/// 造一条 type=1 PostUpsert 事件（fields = parser 提取后的 owned typed 视图，含真 post 字段）。
///
/// HX-C005 后：EventEnvelope 携带 PostFields（不再持 raw JSON）；落库路径 `event_to_upsert_op`
/// 直接读 fields。本助手直接构造 fields，等价于 parser 从 ChannelEvent wire 提取后的结果。
fn post_event(channel: u64, seq: u64, temporary_id: &str, id: &str, text: &str) -> EventEnvelope {
    let fields = PostFields {
        temporary_id: temporary_id.to_string(),
        id: id.to_string(),
        channel_id: cid(channel),
        user_id: "user-zhang".to_string(),
        msg_type: "P".to_string(),
        message: text.to_string(),
        simple_message: text.to_string(),
        props: serde_json::json!({ "channel_event_seq": seq }).to_string(),
        create_at: 1_700_000_000_000_i64 + seq as i64,
        // C2：PostFields 新增 read_bits 字段（type6 已读位 / type1 落库列）。本助手不测已读，留空。
        read_bits: String::new(),
        // A3：viewers（可见性受众）；本助手不测未读门控，留空 Vec。
        viewers: Vec::new(),
        mentions: Vec::new(),
        expedite_map: String::new(),
        user_snapshot: String::new(),
        ..Default::default()
    };
    EventEnvelope::new(channel_id(channel), Seq(seq), EventKind::PostUpsert, fields)
}

async fn scan_message_rows(storage: &HostStorage) -> Vec<helix_core::effect::Row> {
    storage
        .scan(ScanSpec {
            table: "message",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan message table")
}

fn col<'a>(row: &'a helix_core::effect::Row, name: &str) -> Option<&'a SqlValue> {
    row.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn col_text(row: &helix_core::effect::Row, name: &str) -> String {
    match col(row, name) {
        Some(SqlValue::Text(s)) => s.clone(),
        other => panic!("column {name} expected Text, got {other:?}"),
    }
}

/// 核心回归：WS 推送路径用的 `event_to_upsert_op` 落到真 `message` 表（PK=temporary_id）。
///
/// 若回退成写 `messages` 影子表 → execute IM_SCHEMA 不建该表 → batch_upsert 抛
/// `no such table: messages` → 此测试在 batch_upsert 即红。
#[tokio::test]
async fn ws_event_persists_into_678_message_table() {
    let storage = storage_with_im_schema().await;
    let ev = post_event(42, 7, "tmp-abc", "post-real-1", "hello world");

    let op = event_to_upsert_op(&ev);
    storage
        .batch_upsert(match op {
            helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("batch_upsert into message table must succeed (table message exists in IM_SCHEMA)");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(
        rows.len(),
        1,
        "事件必须真落 message 表一行（而非写不存在的 messages）"
    );
    let row = &rows[0];
    assert_eq!(
        col_text(row, "temporary_id"),
        "tmp-abc",
        "PK 取 temporary_id"
    );
    assert_eq!(col_text(row, "id"), "post-real-1");
    assert_eq!(col_text(row, "channel_id"), cid(42));
    assert_eq!(col_text(row, "message"), "hello world");
    assert_eq!(col_text(row, "user_id"), "user-zhang");
    assert_eq!(
        col_text(row, "send_status"),
        "sent",
        "服务端确认的新 WS 行必须持久化为 sent，不能依赖查询层空值回退"
    );
}

/// WS echo 命中本地乐观行时，gate 只覆盖服务端字段，不越权改本地发送状态。
///
/// 实际发送对账随后由 PendingSend::reconcile 显式写 sent 并撤 timer；这里锁住
/// `exclude_from_update=[send_status]`，防止普通重放把 failed/unsend 静默翻回 sent。
#[tokio::test]
async fn ws_event_conflict_preserves_local_send_status_until_reconcile() {
    use helix_core::effect::{StorageOp, UpsertSpec};

    let storage = storage_with_im_schema().await;
    storage
        .batch_upsert(UpsertSpec {
            table: "message",
            rows: vec![vec![
                (
                    "temporary_id".to_string(),
                    SqlValue::Text("tmp-optimistic".to_string()),
                ),
                (
                    "send_status".to_string(),
                    SqlValue::Text("sending".to_string()),
                ),
            ]],
            conflict_key: Some("temporary_id"),
            exclude_from_update: Vec::new(),
        })
        .await
        .expect("seed optimistic row");

    let op = event_to_upsert_op(&post_event(
        42,
        8,
        "tmp-optimistic",
        "post-server-echo",
        "settled body",
    ));
    storage
        .batch_upsert(match op {
            StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("echo upsert must update server fields");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(col_text(&rows[0], "id"), "post-server-echo");
    assert_eq!(
        col_text(&rows[0], "send_status"),
        "sending",
        "gate 冲突更新必须保留本地发送态，终态只由 PendingSend reconcile 决定"
    );
}

/// sync 路径 `batch_upsert_events` 同样真落 message 表（与 WS 路径同表同 PK）。
#[tokio::test]
async fn sync_events_persist_into_678_message_table() {
    let storage = storage_with_im_schema().await;
    let events = vec![
        post_event(42, 10, "tmp-1", "post-1", "first"),
        post_event(42, 11, "tmp-2", "post-2", "second"),
        post_event(42, 12, "tmp-3", "post-3", "third"),
    ];

    for op in helix_im::acl::to_effect::batch_upsert_events(&events) {
        storage
            .batch_upsert(match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
                other => panic!("expected BatchUpsert, got {other:?}"),
            })
            .await
            .expect("sync batch_upsert_events must land in message table");
    }

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 3, "sync 三条事件全部真落 message 表");
}

/// 同一事件 re-sync（续拉重叠批）幂等：conflict_key=temporary_id → 覆盖同行，不产生重复。
/// 这正是修复"cursor 卡住 → 重拉同批"风暴的正确性前提：重拉必须 O(1) 覆盖不膨胀。
#[tokio::test]
async fn resync_same_event_is_idempotent_upsert() {
    let storage = storage_with_im_schema().await;
    let ev = post_event(99, 5, "tmp-dup", "post-dup", "v1");

    for _ in 0..3 {
        let op = event_to_upsert_op(&ev);
        storage
            .batch_upsert(match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
                other => panic!("expected BatchUpsert, got {other:?}"),
            })
            .await
            .expect("idempotent re-upsert");
    }

    let rows = scan_message_rows(&storage).await;
    assert_eq!(
        rows.len(),
        1,
        "重拉同事件必须幂等覆盖（同 PK），不产生重复行"
    );
}

/// 服务端推送无 temporary_id（他人消息）时，PK 回退到 id（真源 cses-client From<Post>），
/// 仍能非空落库，不丢消息。
#[tokio::test]
async fn event_without_temporary_id_falls_back_to_id_as_pk() {
    let storage = storage_with_im_schema().await;
    // 无 temporary_id（他人消息），仅有 id —— event_to_upsert_op 走 PK 回退到 id。
    let fields = PostFields {
        id: "server-post-id-only".to_string(),
        channel_id: cid(7),
        message: "from someone else".to_string(),
        ..Default::default()
    };
    let ev = EventEnvelope::new(channel_id(7), Seq(3), EventKind::PostUpsert, fields);

    let op = event_to_upsert_op(&ev);
    storage
        .batch_upsert(match op {
            helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("upsert with id-as-pk");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        col_text(&rows[0], "temporary_id"),
        "server-post-id-only",
        "无 temporary_id 时回退 id 作 PK"
    );
}

/// G-03a：Native 重启后 pending_sends 为空，失败消息仍能从真 SQLite 恢复并续发 HTTP。
#[tokio::test]
async fn durable_failed_message_rehydrates_retry_after_module_restart() {
    use helix_core::effect::UpsertSpec;
    use helix_core::tick::{AppCommand, PortOutcome};
    use helix_core::{Effect, EffectSink, Module, Tick};
    use helix_driver_host::execute_storage_ops;
    use helix_im::module::ImConfig;
    use helix_im::ImModule;

    let storage = storage_with_im_schema().await;
    storage
        .batch_upsert(UpsertSpec {
            table: "message",
            rows: vec![vec![
                (
                    "temporary_id".to_string(),
                    SqlValue::Text("tmp-restart-retry".to_string()),
                ),
                ("id".to_string(), SqlValue::Text(String::new())),
                ("channel_id".to_string(), SqlValue::Text(cid(1))),
                (
                    "message".to_string(),
                    SqlValue::Text("retry after restart".to_string()),
                ),
                ("type".to_string(), SqlValue::Text("TEXT".to_string())),
                ("props".to_string(), SqlValue::Text("{}".to_string())),
                (
                    "viewers".to_string(),
                    SqlValue::Text("[\"all\"]".to_string()),
                ),
                ("mentions".to_string(), SqlValue::Text("[]".to_string())),
                (
                    "send_status".to_string(),
                    SqlValue::Text("unsend".to_string()),
                ),
                ("create_at".to_string(), SqlValue::Integer(1_234)),
            ]],
            conflict_key: Some("temporary_id"),
            exclude_from_update: Vec::new(),
        })
        .await
        .expect("seed durable failed message");

    let mut module = ImModule::new(ImConfig {
        api_base_url: "http://go.test/api/cses".to_string(),
        auth_user_id: "user-after-restart".to_string(),
        company_id: "team-after-restart".to_string(),
        send_timeout_ms: 15_000,
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let payload = serde_json::to_vec(&serde_json::json!({
        "message_id": "tmp-restart-retry",
        "req_id": "mrc:G-03a:sqlite"
    }))
    .unwrap();
    module
        .handle(
            &Tick::Command(AppCommand::new("im_retry_send", payload)),
            5_000,
            &mut sink,
        )
        .expect("retry command");

    let (lookup_corr, lookup_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("restart retry must query storage");
    let outcome = execute_storage_ops(&storage, &lookup_ops).await;
    assert!(matches!(outcome, PortOutcome::Ok(_)));

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: lookup_corr,
                outcome,
            },
            5_010,
            &mut sink,
        )
        .expect("durable retry reply");

    let http_body = sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Http { req, .. } => req
            .body
            .as_ref()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok()),
        _ => None,
    });
    let http_body = http_body.expect("rehydrated retry must emit posts/create HTTP");
    assert_eq!(http_body["temporaryId"], "tmp-restart-retry");
    assert_eq!(http_body["channelId"], cid(1));
    assert_eq!(http_body["message"], "retry after restart");
    assert!(
        sink.as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::ScheduleTimer { .. })),
        "rehydrated retry must arm a fresh timeout"
    );

    for effect in sink.as_slice() {
        if let Effect::PersistFire { ops } = effect {
            let result = execute_storage_ops(&storage, ops).await;
            assert!(matches!(result, PortOutcome::Ok(_)));
        }
    }
    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        col_text(&rows[0], "send_status"),
        "sending",
        "真实 SQLite 失败行必须在续发时恢复 sending"
    );
}

/// T095：Host SQLite 必须把 closed marker 与 cursor 放在同一笔事务。后续任一写失败时，
/// 先前的 tombstone upsert 必须回滚，不能留下「已关闭但 cursor 未确认」的半状态。
#[tokio::test]
async fn terminal_tombstone_atomic_write_commits_or_rolls_back_as_one_host_transaction() {
    let storage = storage_with_im_schema().await;
    let channel = channel_id(71);
    let terminal = terminal_tombstone_and_cursor_op(channel, Seq(42));
    let failed = execute_storage_ops_atomic(
        &storage,
        &[
            terminal.clone(),
            StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "table_that_does_not_exist",
                key_col: "id",
                key_vals: vec![SqlValue::Text("never".to_string())],
                patch: vec![("state".to_string(), SqlValue::Text("closed".to_string()))],
            }),
        ],
    )
    .await;
    assert!(matches!(failed, helix_core::tick::PortOutcome::Err(_)));
    assert!(
        storage
            .get(GetSpec {
                table: "channel_event_cursor",
                key_col: "channel_id",
                key_val: SqlValue::Text(channel.as_str().to_string()),
            })
            .await
            .expect("rollback verification query")
            .is_none(),
        "failed atomic batch must not leave a terminal marker behind"
    );

    let committed = execute_storage_ops_atomic(&storage, &[terminal]).await;
    assert!(matches!(committed, helix_core::tick::PortOutcome::Ok(_)));
    let row = storage
        .get(GetSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            key_val: SqlValue::Text(channel.as_str().to_string()),
        })
        .await
        .expect("committed cursor query")
        .expect("terminal cursor row");
    assert!(matches!(
        col(&row, "last_event_seq"),
        Some(SqlValue::Integer(42))
    ));
    assert!(matches!(
        col(&row, "terminal_event_seq"),
        Some(SqlValue::Integer(42))
    ));
}

/// GS01-GS04 的 recovery evidence 不能只验证建表。coverage、ledger 与 cursor 以后会被
/// 放进同一 PersistAtomic；这里先锁住 driver 对新增两张业务表的通用事务语义：后续写失败时，
/// 已执行的 coverage/ledger upsert 都不能残留。
#[tokio::test]
async fn recovery_ledger_and_coverage_migrate_idempotently_and_roll_back_together() {
    let storage = storage_with_im_schema().await;
    let coverage = "coverage-rollback-1";
    let ledger = StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_event_ledger",
        rows: vec![vec![
            ("tenant_id".to_string(), SqlValue::Text("team-1".to_string())),
            ("channel_id".to_string(), SqlValue::Text(cid(81))),
            ("event_seq".to_string(), SqlValue::Integer(7)),
            ("event_kind".to_string(), SqlValue::Text("post".to_string())),
            ("message_id".to_string(), SqlValue::Text("post-7".to_string())),
            ("event_hash".to_string(), SqlValue::Text("event-hash-7".to_string())),
            ("coverage_id".to_string(), SqlValue::Text(coverage.to_string())),
            ("applied_at_ms".to_string(), SqlValue::Integer(7_000)),
        ]],
        conflict_key: Some("tenant_id,channel_id,event_seq"),
        exclude_from_update: Vec::new(),
    });
    let coverage_op = StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_sync_coverage",
        rows: vec![vec![
            ("coverage_id".to_string(), SqlValue::Text(coverage.to_string())),
            ("tenant_id".to_string(), SqlValue::Text("team-1".to_string())),
            ("channel_id".to_string(), SqlValue::Text(cid(81))),
            ("from_seq".to_string(), SqlValue::Integer(7)),
            ("to_seq".to_string(), SqlValue::Integer(7)),
            ("event_count".to_string(), SqlValue::Integer(1)),
            ("facts_hash".to_string(), SqlValue::Text("facts-hash-7".to_string())),
            ("correlation_id".to_string(), SqlValue::Text("corr-7".to_string())),
            ("commit_state".to_string(), SqlValue::Text("committed".to_string())),
            ("committed_at_ms".to_string(), SqlValue::Integer(7_000)),
        ]],
        conflict_key: Some("coverage_id"),
        exclude_from_update: Vec::new(),
    });

    let failed = execute_storage_ops_atomic(
        &storage,
        &[
            coverage_op.clone(),
            ledger.clone(),
            StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "table_that_does_not_exist",
                key_col: "id",
                key_vals: vec![SqlValue::Text("rollback".to_string())],
                patch: vec![("state".to_string(), SqlValue::Text("bad".to_string()))],
            }),
        ],
    )
    .await;
    assert!(matches!(failed, helix_core::tick::PortOutcome::Err(_)));
    assert!(storage
        .get(GetSpec {
            table: "channel_sync_coverage",
            key_col: "coverage_id",
            key_val: SqlValue::Text(coverage.to_string()),
        })
        .await
        .expect("coverage rollback query")
        .is_none());
    assert!(storage
        .scan(ScanSpec { table: "channel_event_ledger", limit: None, filter: None, order_by: &[] })
        .await
        .expect("ledger rollback query")
        .is_empty());

    let committed = execute_storage_ops_atomic(&storage, &[coverage_op, ledger]).await;
    assert!(matches!(committed, helix_core::tick::PortOutcome::Ok(_)));
    assert!(storage
        .get(GetSpec {
            table: "channel_sync_coverage",
            key_col: "coverage_id",
            key_val: SqlValue::Text(coverage.to_string()),
        })
        .await
        .expect("coverage committed query")
        .is_some());
}
