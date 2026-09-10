//! 分类接龙 normalized fact 的真实 SQLite migration 与租户隔离回归。
//!
//! 本测试只证明 host 能建立并读回分类接龙的持久化行，不把 SQLite 证据升级为
//! Go/HTTP/WS 或 UI 端到端证据。文字接龙的既有 `chain_*` 表也在同一库内保留验证。

use bytes::Bytes;
use helix_core::effect::{GetSpec, ScanSpec, ScopedGetSpec, SqlValue, StorageOp, UpsertSpec};
use helix_core::ports::Storage;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Effect, EffectSink, Module, Tick};
use helix_driver_host::{
    base64_encode, execute_storage_ops, execute_storage_ops_atomic, HostStorage,
};
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

/// 为每个测试建立独立的 shared-memory tenant 数据库。
async fn storage_with_category_schema() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_category_chain_storage_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open category-chain storage");

    // 先模拟旧 tenant.db：跳过本轮新增 DDL，让后续 migrations 负责补齐它们。
    for &sql in helix_im::IM_SCHEMA {
        if sql.contains("category_chain_") {
            continue;
        }
        storage.execute_raw(sql).await.unwrap_or_else(|error| {
            panic!("execute legacy IM_SCHEMA failed for {sql:?}: {error:?}")
        });
    }
    apply_migrations(&storage).await;
    apply_migrations(&storage).await;
    storage
}

/// 按 host 启动顺序执行 additive migrations，并允许旧列重复迁移。
async fn apply_migrations(storage: &HostStorage) {
    for &sql in helix_im::IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "execute IM_SCHEMA_MIGRATIONS failed for {sql:?}: {error:?}"
            );
        }
    }
}

/// 从持久化行按列读取文本，避免把列顺序误当作 schema 合同。
fn text_column(row: &helix_core::effect::Row, name: &str) -> String {
    match row
        .iter()
        .find(|(column, _)| column == name)
        .map(|(_, value)| value)
    {
        Some(SqlValue::Text(value)) => value.clone(),
        other => panic!("column {name} expected Text, got {other:?}"),
    }
}

/// 返回指定 snapshot_key 下的 canonical leaf，验证 stable key 的复合读路径。
async fn read_fact(
    storage: &HostStorage,
    snapshot_key: &str,
    path: &str,
) -> helix_core::effect::Row {
    storage
        .scoped_get(ScopedGetSpec {
            table: "category_chain_fact",
            scope_col: "snapshot_key",
            scope_val: SqlValue::Text(snapshot_key.to_string()),
            key_col: "path",
            key_val: SqlValue::Text(path.to_string()),
        })
        .await
        .unwrap_or_else(|error| panic!("read category fact {snapshot_key}/{path}: {error:?}"))
        .unwrap_or_else(|| panic!("category fact {snapshot_key}/{path} missing"))
}

/// 写入一个 head 指针，保证 revision 始终按服务端十进制字符串保存。
async fn write_head(storage: &HostStorage, scope: &str, revision: &str, snapshot_key: &str) {
    storage
        .batch_upsert(UpsertSpec::new(
            "category_chain_head",
            vec![vec![
                ("scope".to_string(), SqlValue::Text(scope.to_string())),
                ("revision".to_string(), SqlValue::Text(revision.to_string())),
                (
                    "snapshot_key".to_string(),
                    SqlValue::Text(snapshot_key.to_string()),
                ),
            ]],
            Some("scope"),
        ))
        .await
        .expect("category head upsert");
}

/// 写入一批有界 canonical leaf，使用复合主键做 O(1) 单行替换。
async fn write_facts(
    storage: &HostStorage,
    snapshot_key: &str,
    revision: &str,
    leaves: &[(&str, &str)],
) {
    let rows = leaves
        .iter()
        .map(|(path, value_json)| {
            vec![
                (
                    "snapshot_key".to_string(),
                    SqlValue::Text(snapshot_key.to_string()),
                ),
                ("path".to_string(), SqlValue::Text((*path).to_string())),
                ("revision".to_string(), SqlValue::Text(revision.to_string())),
                (
                    "value_json".to_string(),
                    SqlValue::Text((*value_json).to_string()),
                ),
            ]
        })
        .collect();
    storage
        .batch_upsert(UpsertSpec::new(
            "category_chain_fact",
            rows,
            Some("snapshot_key,path"),
        ))
        .await
        .expect("category facts upsert");
}

/// 验证 additive migration、head/fact 读回以及文字接龙表共存。
#[tokio::test]
async fn category_chain_schema_migrates_and_preserves_text_chain_tables() {
    let storage = storage_with_category_schema().await;

    storage
        .batch_upsert(UpsertSpec::new(
            "chain_summary",
            vec![vec![
                (
                    "chain_id".to_string(),
                    SqlValue::Text("text-chain".to_string()),
                ),
                (
                    "channel_id".to_string(),
                    SqlValue::Text("channel-text".to_string()),
                ),
                ("mode".to_string(), SqlValue::Text("TEXT".to_string())),
                (
                    "status".to_string(),
                    SqlValue::Text("PUBLISHED".to_string()),
                ),
            ]],
            Some("chain_id"),
        ))
        .await
        .expect("existing text-chain summary remains writable");

    let snapshot_key =
        "[\"account-1\",\"tenant-1\",\"channel-category\",\"self\",\"category-chain\"]";
    write_head(&storage, "viewer:user-1", "7", snapshot_key).await;
    write_facts(
        &storage,
        snapshot_key,
        "7",
        &[
            ("selectedCategories[0]", "\"one\""),
            ("selectedCategories[1]", "\"two\""),
            ("selectedCategories[2]", "\"three\""),
        ],
    )
    .await;

    let head = storage
        .get(GetSpec {
            table: "category_chain_head",
            key_col: "scope",
            key_val: SqlValue::Text("viewer:user-1".to_string()),
        })
        .await
        .expect("read category head")
        .expect("category head exists");
    assert_eq!(text_column(&head, "revision"), "7");
    assert_eq!(text_column(&head, "snapshot_key"), snapshot_key);
    let first = read_fact(&storage, snapshot_key, "selectedCategories[0]").await;
    assert_eq!(text_column(&first, "revision"), "7");
    assert_eq!(text_column(&first, "value_json"), "\"one\"");

    let text_chain = storage
        .get(GetSpec {
            table: "chain_summary",
            key_col: "chain_id",
            key_val: SqlValue::Text("text-chain".to_string()),
        })
        .await
        .expect("read existing text-chain summary")
        .expect("text-chain summary remains present");
    assert_eq!(text_column(&text_chain, "mode"), "TEXT");
}

/// 验证较短的新 revision 不会让旧数组尾 leaf 复活，且旧行不会被整组 RMW 删除。
#[tokio::test]
async fn shorter_revision_filters_stale_fact_tail_without_array_rewrite() {
    let storage = storage_with_category_schema().await;
    let snapshot_key =
        "[\"account-1\",\"tenant-1\",\"channel-category\",\"self\",\"category-chain\"]";
    write_head(&storage, "viewer:user-1", "7", snapshot_key).await;
    write_facts(
        &storage,
        snapshot_key,
        "7",
        &[
            ("selectedCategories[0]", "\"one\""),
            ("selectedCategories[1]", "\"two\""),
            ("selectedCategories[2]", "\"three\""),
        ],
    )
    .await;

    // 新快照只有一个 leaf；head 切换与 leaf upsert 是独立可回灌的持久事实。
    write_head(&storage, "viewer:user-1", "8", snapshot_key).await;
    write_facts(
        &storage,
        snapshot_key,
        "8",
        &[("selectedCategories[0]", "\"one-updated\"")],
    )
    .await;

    let rows = storage
        .scan(ScanSpec {
            table: "category_chain_fact",
            limit: None,
            filter: Some(("snapshot_key", SqlValue::Text(snapshot_key.to_string()))),
            order_by: &[],
        })
        .await
        .expect("scan all facts for snapshot");
    assert_eq!(rows.len(), 3, "old tail remains as historical rows");
    let current: Vec<_> = rows
        .iter()
        .filter(|row| text_column(row, "revision") == "8")
        .collect();
    assert_eq!(current.len(), 1);
    assert_eq!(text_column(current[0], "path"), "selectedCategories[0]");
    assert_eq!(text_column(current[0], "value_json"), "\"one-updated\"");
    assert!(rows.iter().any(|row| {
        text_column(row, "path") == "selectedCategories[1]" && text_column(row, "revision") == "7"
    }));
    assert!(rows.iter().any(|row| {
        text_column(row, "path") == "selectedCategories[2]" && text_column(row, "revision") == "7"
    }));
}

/// 验证相同 scope/snapshot_key 在两个 tenant.db 中互不泄漏。
#[tokio::test]
async fn category_chain_storage_isolated_between_tenants() {
    let first = storage_with_category_schema().await;
    let second = storage_with_category_schema().await;
    let snapshot_key = "[\"account\",\"tenant\",\"channel\",\"self\",\"chain\"]";
    write_head(&first, "viewer:user", "11", snapshot_key).await;
    write_head(&second, "viewer:user", "12", snapshot_key).await;
    write_facts(&first, snapshot_key, "11", &[("status", "\"OPEN\"")]).await;
    write_facts(&second, snapshot_key, "12", &[("status", "\"CLOSED\"")]).await;

    let first_head = first
        .get(GetSpec {
            table: "category_chain_head",
            key_col: "scope",
            key_val: SqlValue::Text("viewer:user".to_string()),
        })
        .await
        .expect("read first tenant")
        .expect("first tenant head");
    let second_head = second
        .get(GetSpec {
            table: "category_chain_head",
            key_col: "scope",
            key_val: SqlValue::Text("viewer:user".to_string()),
        })
        .await
        .expect("read second tenant")
        .expect("second tenant head");
    assert_eq!(text_column(&first_head, "revision"), "11");
    assert_eq!(text_column(&second_head, "revision"), "12");
    assert_eq!(
        text_column(
            &read_fact(&first, snapshot_key, "status").await,
            "value_json"
        ),
        "\"OPEN\""
    );
    assert_eq!(
        text_column(
            &read_fact(&second, snapshot_key, "status").await,
            "value_json"
        ),
        "\"CLOSED\""
    );
}

/// 生成与公开 category-chain lifecycle seam 相同形态的真实 channel id。
fn lifecycle_channel_id() -> String {
    "chfixx00000000000000000001".to_string()
}

/// 生成可被现有 message parser 接受的 canonical anchor post id。
fn lifecycle_post_id() -> String {
    "srvfix00000000000000000001".to_string()
}

/// 构造完整共享卡片；post.props.categoryChain 与此值保持逐字一致。
fn lifecycle_card(revision: &str) -> Value {
    json!({
        "schemaVersion": 1,
        "chainId": "chain-a",
        "revision": revision,
        "status": "OPEN",
        "title": "活动",
        "description": "",
        "example": "",
        "deadlineAt": 1789113600000_i64,
        "audienceCount": 5,
        "participantCount": 1,
        "categoryCount": 2,
        "maxCategories": 2,
        "categories": [
            {"categoryId":"a","name":"室内","ordinal":0,"tone":"purple","revision":revision,"participantCount":1,"entries":[],"hasMore":true},
            {"categoryId":"b","name":"户外","ordinal":1,"tone":"blue","revision":revision,"participantCount":1,"entries":[],"hasMore":true}
        ]
    })
}

/// 构造真实 CanonicalPost；runtime 会把它编译成 message 表的 readback upsert。
fn lifecycle_post(revision: &str) -> Value {
    json!({
        "id": lifecycle_post_id(),
        "temporaryId": "category-anchor-1",
        "channelId": lifecycle_channel_id(),
        "userId": "user-a",
        "type": "CATEGORY_CHAIN",
        "message": "分类接龙：活动",
        "simpleMessage": "[分类接龙] 活动",
        "viewers": ["all"],
        "props": {"categoryChain": lifecycle_card(revision)},
        "createAt": 1788770000000_i64,
        "updateAt": 1788770000000_i64
    })
}

/// 构造 set-participation 的服务端确认结果，保留 card、post 和本人选择集。
fn lifecycle_mutation(revision: &str, mutation_id: &str) -> Value {
    json!({
        "outcome": "CONFIRMED",
        "commandKind": "set-participation",
        "clientMutationId": mutation_id,
        "operationId": format!("op-{mutation_id}"),
        "chainId": "chain-a",
        "revision": revision,
        "eventId": null,
        "eventSeq": null,
        "draft": null,
        "card": lifecycle_card(revision),
        "post": lifecycle_post(revision),
        "myParticipation": {
            "chainId": "chain-a",
            "participationRevision": "1",
            "maxCategories": 2,
            "entries": [],
            "canParticipate": true,
            "canEdit": true,
            "canCancel": true,
            "denialReason": null
        }
    })
}

/// 解析当前 sink 中已经产生的 category operation 终态事件。
fn confirmed_category_events(sink: &EffectSink) -> Vec<Value> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(&event.0).ok(),
            _ => None,
        })
        .filter(|event| {
            event["event"] == "im:category_chain:operation" && event["data"]["state"] == "CONFIRMED"
        })
        .collect()
}

/// 构造 host 实际回灌的 ADR-007 HTTP 信封；业务 CommonRes 只位于 base64 body。
fn category_http_success(data: Value) -> PortOutcome {
    let body = serde_json::to_vec(&json!({"status":"SUCCESS", "data":data}))
        .expect("category CommonRes JSON");
    let envelope = serde_json::to_vec(&json!({
        "status": 200,
        "headers": {},
        "body": base64_encode(&body)
    }))
    .expect("category ADR-007 envelope JSON");
    PortOutcome::Ok(ReplyBytes(Bytes::from(envelope)))
}

/// 真 SQLite 驱动公开 ImModule：HTTP 回包只推进到 Persist，Atomic/Readback 全部经 HostStorage。
#[tokio::test]
async fn category_chain_module_reaches_terminal_only_after_sqlite_persist_and_readback() {
    let storage = storage_with_category_schema().await;
    let mut module = ImModule::new(ImConfig {
        api_base_url: "http://go.test/api/cses".to_string(),
        auth_user_id: "user-a".to_string(),
        company_id: "tenant-a".to_string(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let command = helix_im::client_api::build_command(
        "im_category_chain_set_participation",
        &serde_json::to_vec(&json!({
            "channelId": lifecycle_channel_id(),
            "chainId": "chain-a",
            "expectedParticipationRevision": "0",
            "selections": [],
            "clientMutationId": "sqlite-lifecycle"
        }))
        .expect("category command JSON"),
    )
    .expect("public category command");
    module
        .handle(&Tick::Command(command), 1_000, &mut sink)
        .expect("set-participation command");

    let http_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("category command must issue HTTP");
    assert!(confirmed_category_events(&sink).is_empty());

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: http_corr,
                outcome: category_http_success(lifecycle_mutation("7", "sqlite-lifecycle")),
            },
            1_001,
            &mut sink,
        )
        .expect("HTTP authority reply");
    assert!(
        confirmed_category_events(&sink).is_empty(),
        "HTTP success must not confirm before storage"
    );

    let mut saw_atomic = false;
    let mut reached_terminal = false;
    for step in 0..32_u64 {
        let pending = sink.as_slice().iter().find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone(), false)),
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone(), true)),
            _ => None,
        });
        let Some((corr, ops, atomic)) = pending else {
            reached_terminal = !confirmed_category_events(&sink).is_empty();
            break;
        };
        assert!(
            confirmed_category_events(&sink).is_empty(),
            "Persist/readback must precede the terminal confirmation"
        );
        let stage = if atomic {
            "PersistAtomic"
        } else if ops.iter().any(|op| matches!(op, StorageOp::Scan(_))) {
            "Readback"
        } else {
            "Persist"
        };
        let outcome = if atomic {
            saw_atomic = true;
            execute_storage_ops_atomic(&storage, &ops).await
        } else {
            execute_storage_ops(&storage, &ops).await
        };
        assert!(
            matches!(&outcome, PortOutcome::Ok(_)),
            "real SQLite {stage} failed: {outcome:?}; ops={ops:?}"
        );
        sink.clear();
        module
            .handle(&Tick::PortReply { corr, outcome }, 2_000 + step, &mut sink)
            .expect("storage PortReply");
        if !confirmed_category_events(&sink).is_empty() {
            reached_terminal = true;
            break;
        }
    }
    assert!(saw_atomic, "authority write must use PersistAtomic");
    assert!(
        reached_terminal,
        "SQLite readback must release terminal EventSink"
    );

    let terminal = confirmed_category_events(&sink);
    assert_eq!(terminal.len(), 1, "one mutation must have one confirmation");
    assert_eq!(terminal[0]["data"]["operationId"], "op-sqlite-lifecycle");
    assert_eq!(terminal[0]["data"]["card"]["revision"], "7");

    let messages = storage
        .scan(ScanSpec {
            table: "message",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .expect("scan durable message");
    assert_eq!(
        messages.len(),
        1,
        "canonical post must be durable exactly once"
    );
    assert_eq!(
        text_column(&messages[0], "temporary_id"),
        "category-anchor-1"
    );
    assert_eq!(text_column(&messages[0], "id"), lifecycle_post_id());
    assert_eq!(
        text_column(&messages[0], "channel_id"),
        lifecycle_channel_id()
    );
    assert_eq!(text_column(&messages[0], "type"), "CATEGORY_CHAIN");

    let card_scope = json!(["user-a", lifecycle_channel_id(), "chain-a", "card"]).to_string();
    let card_facts = storage
        .scan(ScanSpec {
            table: "category_chain_fact",
            limit: None,
            filter: Some(("snapshot_key", SqlValue::Text(card_scope.clone()))),
            order_by: &[],
        })
        .await
        .expect("scan durable category facts");
    assert!(
        card_facts.iter().any(|row| {
            text_column(row, "path") == "/card/revision"
                && text_column(row, "revision") == "7"
                && text_column(row, "value_json") == "\"7\""
        }),
        "card revision must be read back from normalized facts"
    );
    let card_head = storage
        .get(GetSpec {
            table: "category_chain_head",
            key_col: "scope",
            key_val: SqlValue::Text(card_scope),
        })
        .await
        .expect("read category card head")
        .expect("category card head exists");
    assert_eq!(text_column(&card_head, "revision"), "7");
}

/// Execute only local storage effects, returning every public terminal emitted by the module.
async fn drive_review_storage(
    module: &mut ImModule,
    storage: &HostStorage,
    mut sink: EffectSink,
) -> Vec<Value> {
    let mut events = Vec::new();
    for step in 0..64 {
        let mut replies = Vec::new();
        for effect in sink.as_slice() {
            match effect {
                Effect::Persist { corr, ops } => {
                    replies.push((*corr, execute_storage_ops(storage, ops).await))
                }
                Effect::PersistAtomic { corr, ops } => {
                    replies.push((*corr, execute_storage_ops_atomic(storage, ops).await))
                }
                Effect::PersistFire { ops } => {
                    assert!(matches!(
                        execute_storage_ops(storage, ops).await,
                        PortOutcome::Ok(_)
                    ));
                }
                Effect::Emit { event } => {
                    events.push(serde_json::from_slice(event.0.as_ref()).expect("event JSON"))
                }
                _ => {}
            }
        }
        if replies.is_empty() {
            return events;
        }
        sink.clear();
        for (corr, outcome) in replies {
            assert!(
                matches!(&outcome, PortOutcome::Ok(_)),
                "storage failed: {outcome:?}"
            );
            module
                .handle(&Tick::PortReply { corr, outcome }, 2000 + step, &mut sink)
                .expect("storage reply");
        }
    }
    panic!("storage continuations did not terminate");
}

#[tokio::test]
/// A delayed ordinary Post cannot overwrite a newer HTTP snapshot or emit stale category content.
async fn category_review_http_then_older_post_keeps_durable_revision() {
    let storage = storage_with_category_schema().await;
    let mut module = ImModule::new(ImConfig {
        api_base_url: "http://go.test/api/cses".into(),
        auth_user_id: "user-a".into(),
        company_id: "tenant-a".into(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let command = helix_im::client_api::build_command(
        "im_category_chain_set_participation",
        &serde_json::to_vec(
            &json!({"channelId":lifecycle_channel_id(),"chainId":"chain-a",
            "expectedParticipationRevision":"0","selections":[],"clientMutationId":"review"}),
        )
        .unwrap(),
    )
    .unwrap();
    module
        .handle(&Tick::Command(command), 1000, &mut sink)
        .unwrap();
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|fx| {
            if let Effect::Http { corr, .. } = fx {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: category_http_success(lifecycle_mutation("10", "review")),
            },
            1001,
            &mut sink,
        )
        .unwrap();
    let events = drive_review_storage(&mut module, &storage, sink).await;
    assert!(events.iter().any(|e| e["data"]["state"] == "CONFIRMED"));
    for (seq, action) in [
        (1, "post"),
        (2, "post_update"),
        (3, "posts_update"),
        (5, "post_update"),
        (4, "post"),
    ] {
        let mut post = lifecycle_post("9");
        post["event_seq"] = json!(seq);
        let frame = if action == "posts_update" {
            json!({"action":action,"data":{"events":[{"post":post,"event_seq":seq}]}})
        } else {
            json!({"action":action,"data":post,"event_seq":seq})
        };
        let mut sink = EffectSink::new();
        module
            .handle(
                &Tick::Inbound(helix_core::tick::InboundBytes(Bytes::from(
                    serde_json::to_vec(&frame).unwrap(),
                ))),
                3000 + seq,
                &mut sink,
            )
            .unwrap();
        let events = drive_review_storage(&mut module, &storage, sink).await;
        let saved = storage
            .get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: SqlValue::Text("category-anchor-1".into()),
            })
            .await
            .unwrap()
            .unwrap();
        let props: Value = serde_json::from_str(&text_column(&saved, "props")).unwrap();
        assert_eq!(
            props["categoryChain"]["revision"], "10",
            "{action} regressed durable state"
        );
        assert!(
            seq == 5
                || events.iter().any(|event| event
                    .pointer("/data/props/categoryChain/revision")
                    .or_else(|| event.pointer("/data/posts/0/props/categoryChain/revision"))
                    .is_some()),
            "{action} emitted no post"
        );
        for event in events {
            if let Some(revision) = event
                .pointer("/data/props/categoryChain/revision")
                .or_else(|| event.pointer("/data/posts/0/props/categoryChain/revision"))
            {
                assert_eq!(revision, "10", "{action} emitted stale content");
            }
        }
    }
}

/// Existing tenant facts receive the indexed version key before a bounded read, without changing TEXT rows.
#[tokio::test]
async fn category_review_migrates_legacy_fact_and_message_versions() {
    let sequence = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let storage = HostStorage::open_sqlite_url(&format!(
        "file:category_review_migrate_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        sequence
    ))
    .await
    .unwrap();
    for sql in [
        "CREATE TABLE category_chain_fact(snapshot_key TEXT, path TEXT, revision TEXT, value_json TEXT, PRIMARY KEY(snapshot_key,path))",
        "INSERT INTO category_chain_fact VALUES ('scope','/card/revision','10','\"10\"')",
        "CREATE TABLE message(temporary_id TEXT PRIMARY KEY, type TEXT, props TEXT)",
        "INSERT INTO message VALUES ('category','CATEGORY_CHAIN','{\"categoryChain\":{\"revision\":\"18446744073709551615\"}}'), ('text','TEXT','{}')",
    ] { storage.execute_raw(sql).await.unwrap(); }
    for _ in 0..2 {
        for sql in helix_im::IM_SCHEMA_MIGRATIONS
            .iter()
            .filter(|sql| sql.contains("version_key") || sql.contains("category_revision"))
        {
            if let Err(error) = storage.execute_raw(sql).await {
                assert!(
                    error.to_string().contains("duplicate column name"),
                    "migration failed: {error}"
                );
            }
        }
    }
    let row = read_fact(&storage, "scope", "/card/revision").await;
    assert_eq!(text_column(&row, "version_key"), "scope:10");
    for (id, expected) in [("category", "18446744073709551615"), ("text", "")] {
        let row = storage
            .get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: SqlValue::Text(id.into()),
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text_column(&row, "category_revision"), expected);
    }
}
