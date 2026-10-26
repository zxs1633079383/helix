//! 两个列表的精确空串迁移与新旧数据库写入兼容；内存库由测试独占。
use rusqlite::Connection;

/// 执行启动同款 schema/migration，仅容忍重复 additive 列。
fn migrate(db: &Connection) {
    for sql in helix_im::IM_SCHEMA
        .iter()
        .chain(helix_im::IM_SCHEMA_MIGRATIONS)
    {
        if let Err(e) = db.execute_batch(sql) {
            assert!(
                e.to_string().contains("duplicate column name"),
                "{sql}: {e}"
            );
        }
    }
}

/// 有效列表和损坏非空数据原样保留，重复启动无副作用，新行不得继续产生空串。
#[test]
fn member_lists_migrate_exact_empty_and_preserve_other_values() {
    let db = Connection::open_in_memory().unwrap();
    // 模拟旧版表默认值；迁移前写入现场同款空串，避免触发器提前修复掩盖迁移缺陷。
    for sql in helix_im::IM_SCHEMA {
        db.execute_batch(
            &sql.replace(
                "mention_list TEXT NOT NULL DEFAULT '[]'",
                "mention_list TEXT NOT NULL DEFAULT ''",
            )
            .replace(
                "urgent_post_list TEXT NOT NULL DEFAULT '[]'",
                "urgent_post_list TEXT NOT NULL DEFAULT ''",
            ),
        )
        .unwrap();
    }
    db.execute_batch("INSERT INTO channel_member(channel_id,user_id,mention_list,urgent_post_list) VALUES ('legacy','u','',''), ('valid','u','[\"m\"]','[\"p\"]'), ('bad','u','broken',' ')").unwrap();
    db.execute_batch("INSERT INTO channel(id,mention_list,urgent_post_list) VALUES ('migrate-channel','','[\"keep\"]')").unwrap();
    for _ in 0..2 {
        migrate(&db);
    }
    let migrated_channel: (String, String) = db
        .query_row(
            "SELECT mention_list,urgent_post_list FROM channel WHERE id='migrate-channel'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(migrated_channel, ("[]".into(), "[\"keep\"]".into()));
    let get = |id: &str| -> (String, String) {
        db.query_row(
            "SELECT mention_list,urgent_post_list FROM channel_member WHERE channel_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(get("legacy"), ("[]".into(), "[]".into()));
    assert_eq!(get("valid"), ("[\"m\"]".into(), "[\"p\"]".into()));
    assert_eq!(get("bad"), ("broken".into(), " ".into()));
    db.execute_batch("INSERT INTO channel_member(channel_id,user_id) VALUES ('new','u')")
        .unwrap();
    assert_eq!(get("new"), ("[]".into(), "[]".into()));
    db.execute_batch("UPDATE channel_member SET mention_list='',urgent_post_list='[\"keep\"]' WHERE channel_id='new'; INSERT INTO channel(id,mention_list,urgent_post_list) VALUES ('old-channel','','[\"keep\"]')").unwrap();
    assert_eq!(get("new"), ("[]".into(), "[\"keep\"]".into()));
    let channel: (String, String) = db
        .query_row(
            "SELECT mention_list,urgent_post_list FROM channel WHERE id='old-channel'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(channel, ("[]".into(), "[\"keep\"]".into()));
    let fresh = Connection::open_in_memory().unwrap();
    migrate(&fresh);
    for table in ["channel", "channel_member"] {
        let mut stmt = fresh
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        let defaults: Vec<(String, Option<String>)> = stmt
            .query_map([], |r| Ok((r.get(1)?, r.get(4)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for col in ["mention_list", "urgent_post_list"] {
            assert_eq!(
                defaults
                    .iter()
                    .find(|(name, _)| name == col)
                    .unwrap()
                    .1
                    .as_deref(),
                Some("'[]'")
            );
        }
    }
}

#[path = "../../helix-im/tests/common.rs"]
mod common;

/// 公开 post 入站的真实事务、成员读回及 ACK 意图逐条关联；无真实 HTTP/macOS 展示。
#[tokio::test]
async fn ten_posts_persist_and_emit_once_with_empty_member_lists() {
    use helix_core::effect::{GetSpec, SqlValue};
    use helix_core::ports::Storage;
    use helix_core::tick::{InboundBytes, PortOutcome};
    use helix_core::{Effect, EffectSink, Module, Tick};
    use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
    use helix_im::{ImConfig, ImModule};
    use serde_json::{json, Value};
    let db = HostStorage::open_sqlite_url("sqlite::memory:")
        .await
        .unwrap();
    for sql in helix_im::IM_SCHEMA
        .iter()
        .chain(helix_im::IM_SCHEMA_MIGRATIONS)
    {
        if let Err(e) = db.execute_raw(sql).await {
            assert!(e.to_string().contains("duplicate column name"), "{e}");
        }
    }
    let channel = common::cid(1);
    db.batch_upsert(helix_core::effect::UpsertSpec::new(
        "channel",
        vec![vec![
            ("id".into(), SqlValue::Text(channel.clone())),
            ("user_id".into(), SqlValue::Text("receiver".into())),
            ("team_id".into(), SqlValue::Text("company".into())),
        ]],
        Some("id"),
    ))
    .await
    .unwrap();
    db.batch_upsert(helix_core::effect::UpsertSpec::new(
        "channel_member",
        vec![vec![
            ("channel_id".into(), SqlValue::Text(channel.clone())),
            ("user_id".into(), SqlValue::Text("receiver".into())),
            ("last_post".into(), SqlValue::Text("{}".into())),
        ]],
        Some("channel_id,user_id"),
    ))
    .await
    .unwrap();
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "receiver".into(),
        company_id: "company".into(),
        ..Default::default()
    });
    module.register_channel(common::channel_id(1), 0);
    // 通过公开离线查询读取真实频道行，建立公司/用户围栏，不能绕过权威归属检查。
    let mut startup = EffectSink::new();
    module
        .handle(
            &Tick::Command(helix_core::tick::AppCommand::new(
                "im_query_dialog_list",
                br#"{"req_id":"member-startup"}"#.to_vec(),
            )),
            0,
            &mut startup,
        )
        .unwrap();
    let startup_batch = std::mem::take(&mut startup);
    for effect in startup_batch.as_slice() {
        if let Effect::Persist { corr, ops } = effect {
            let outcome = execute_storage_ops(&db, ops).await;
            assert!(matches!(outcome, PortOutcome::Ok(_)));
            module
                .handle(
                    &Tick::PortReply {
                        corr: *corr,
                        outcome,
                    },
                    0,
                    &mut startup,
                )
                .unwrap();
        }
    }
    for seq in 1..=10 {
        let id = common::sid(seq);
        let mut sink = EffectSink::new();
        module.handle(&Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"post","data":{"channelId":channel,"event_seq":seq,"post":{"id":id,"temporaryId":id,"channelId":channel,"userId":"sender","type":"TEXT","message":"member regression","simpleMessage":"member regression","props":{},"viewers":["all"],"createAt":seq,"updateAt":seq}}})).unwrap().into())), seq, &mut sink).unwrap();
        assert!(!sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. } | Effect::Http { .. })));
        let mut events: Vec<Value> = Vec::new();
        let mut acks: Vec<Value> = Vec::new();
        for _ in 0..12 {
            if sink.is_empty() {
                break;
            }
            let batch = std::mem::take(&mut sink);
            for effect in batch.as_slice() {
                let (corr, outcome) = match effect {
                    Effect::PersistAtomic { corr, ops } => {
                        (*corr, execute_storage_ops_atomic(&db, ops).await)
                    }
                    Effect::Persist { corr, ops } => (*corr, execute_storage_ops(&db, ops).await),
                    Effect::Emit { event } => {
                        events.push(serde_json::from_slice(&event.0).unwrap());
                        continue;
                    }
                    Effect::Http { corr, req } if req.url.ends_with("/post/clientAck") => {
                        acks.push(serde_json::from_slice(req.body.as_ref().unwrap()).unwrap());
                        // 隔离 HTTP 回包仅证明 ACK 生命周期，不发送真实网络请求。
                        (
                            *corr,
                            PortOutcome::Ok(helix_core::tick::ReplyBytes(common::enveloped(
                                bytes::Bytes::from_static(
                                    br#"{"status":"SUCCESS","data":{"recorded":true}}"#,
                                ),
                            ))),
                        )
                    }
                    Effect::Http { req, .. } => panic!("unexpected HTTP {}", req.url),
                    _ => continue,
                };
                assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
                module
                    .handle(&Tick::PortReply { corr, outcome }, seq, &mut sink)
                    .unwrap();
            }
        }
        assert!(sink.is_empty(), "bounded lifecycle");
        assert_eq!(acks.len(), 1);
        assert_eq!(acks[0]["postId"], id);
        assert_eq!(acks[0]["ackId"], format!("{id}:{seq}"));
        let received: Vec<_> = events
            .iter()
            .filter(|e| e["event"] == "im:post:received")
            .collect();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["data"]["id"], id);
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "im:channel:update")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e["event"] == "im:post:client-ack-succeeded")
                .count(),
            1
        );
        assert!(db
            .get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: SqlValue::Text(id)
            })
            .await
            .unwrap()
            .is_some());
    }
    // 本轮唯一内存库 drop 即清理，不接触账户库或远端业务数据。
}
