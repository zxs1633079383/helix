//! 群聊空值回归：公开 ImModule 入站 → 真实 SQLite → 生产回读/Emit。
#[path = "../../helix-im/tests/common.rs"]
mod common;
use common::{cid, Harness};
use helix_core::effect::{Row, ScanSpec, SqlValue};
use helix_core::ports::Storage;
use helix_core::tick::{InboundBytes, PortOutcome};
use helix_core::{Effect, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
static SEQ: AtomicUsize = AtomicUsize::new(0);

/// 每例独立内存数据库，使用产品启动时的表与迁移。
async fn database() -> HostStorage {
    let url = format!(
        "file:channel_empty_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let db = HostStorage::open_sqlite_url(&url).await.unwrap();
    for sql in helix_im::IM_SCHEMA {
        db.execute_raw(sql).await.unwrap();
    }
    for sql in helix_im::IM_SCHEMA_MIGRATIONS {
        if let Err(error) = db.execute_raw(sql).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "{error:?}"
            );
        }
    }
    db
}

/// 执行所有生产存储效果并反馈真实结果，禁止以模拟成功掩盖 SQL 错误。
async fn deliver(db: &HostStorage, h: &mut Harness, action: &str, data: Value) -> Vec<Value> {
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action": action, "data": data}))
                .unwrap()
                .into(),
        )),
        1000,
    );
    let mut pending = std::collections::VecDeque::new();
    let mut events = Vec::new();
    for _ in 0..24 {
        for effect in h.sink.as_slice() {
            match effect {
                Effect::PersistAtomic { corr, ops } => {
                    let outcome = execute_storage_ops_atomic(db, ops).await;
                    assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
                    pending.push_back(Tick::PortReply {
                        corr: *corr,
                        outcome,
                    });
                }
                Effect::Persist { corr, ops } => {
                    let outcome = execute_storage_ops(db, ops).await;
                    assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
                    pending.push_back(Tick::PortReply {
                        corr: *corr,
                        outcome,
                    });
                }
                Effect::PersistFire { ops } => assert!(matches!(
                    execute_storage_ops(db, ops).await,
                    PortOutcome::Ok(_)
                )),
                Effect::Emit { event } => events.push(serde_json::from_slice(&event.0).unwrap()),
                Effect::Http { .. } | Effect::HttpFire { .. } => {
                    panic!("unexpected network dependency")
                }
                _ => {}
            }
        }
        let Some(reply) = pending.pop_front() else {
            return events;
        };
        h.step(reply, 1001);
    }
    panic!("storage reply loop did not settle");
}

/// 直接读取实际表，不复用被测映射实现。
async fn rows(db: &HostStorage, table: &'static str) -> Vec<Row> {
    db.scan(ScanSpec {
        table,
        limit: None,
        filter: None,
        order_by: &[],
    })
    .await
    .unwrap()
}

/// 从 SQLite 命名列读取值。
fn value(row: &Row, key: &str) -> Value {
    match &row.iter().find(|(name, _)| name == key).unwrap().1 {
        SqlValue::Text(value) => json!(value),
        SqlValue::Integer(value) => json!(value),
        other => panic!("unexpected column {key}: {other:?}"),
    }
}

/// 公共生产 builder 的完整输入加上频道事实，建立有值基线。
fn initial() -> Value {
    let wire: Value = serde_json::from_str(include_str!(
        "../../helix-im/tests/channel_empty_semantics_go_input.json"
    ))
    .unwrap();
    let mut data = wire["data"].clone();
    for (key, value) in json!({"type":"P","displayName":"旧群名","purpose":"旧简介","orient":"旧说明","picture":{"url":"avatar"},"props":{"notice":"公告"},"channelIsTop":true,"mentionCount":7}).as_object().unwrap() {
        data[key] = value.clone();
    }
    data
}

/// 历史 join 不降级当前管理员；角色摘要不抹昵称；Emit 来自真实落库名单。
#[tokio::test]
async fn current_roster_survives_history_in_database_and_emit() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    let events = deliver(&db, &mut h, "channel_member_update", initial()).await;
    let members = rows(&db, "channel_member").await;
    let manager = members
        .iter()
        .find(|row| value(row, "user_id") == json!("manager"))
        .unwrap();
    assert_eq!(value(manager, "role"), json!("ADMIN"));
    let owner = members
        .iter()
        .find(|row| value(row, "user_id") == json!("owner"))
        .unwrap();
    assert_eq!(value(owner, "nick_name"), json!("群主昵称"));
    let event = events
        .iter()
        .find(|event| event["event"] == "im:channel:member-updated")
        .expect("durable roster event");
    assert_eq!(
        event["data"]["channel"]["adminUsers"][0]["id"], "manager",
        "{event}"
    );
}

/// 缺失/null 保留旧值，不同 presence 的成员共帧不发生列错位。
#[tokio::test]
async fn sparse_and_null_fields_preserve_stored_values() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    deliver(&db, &mut h, "channel_member_update", initial()).await;
    let before = rows(&db, "channel").await.remove(0);
    deliver(&db, &mut h, "channel_member_update", json!({"id":cid(1),"props":null,"picture":null,"purpose":null,"members":[{"id":"owner"},{"id":"manager","nickName":"改昵称"},{"id":"new-member","role":"MEMBER","nickName":""}]})).await;
    let after = rows(&db, "channel").await.remove(0);
    for field in [
        "display_name",
        "purpose",
        "orient",
        "props",
        "picture",
        "is_top",
        "mention_count",
        "role",
        "type",
    ] {
        assert_eq!(value(&after, field), value(&before, field), "{field}");
    }
    let members = rows(&db, "channel_member").await;
    let manager = members
        .iter()
        .find(|row| value(row, "user_id") == json!("manager"))
        .unwrap();
    assert_eq!(value(manager, "role"), json!("ADMIN"));
    assert_eq!(value(manager, "team_id"), json!("team-a"));
    assert_eq!(value(manager, "nick_name"), json!("改昵称"));
    let events = deliver(&db, &mut h, "update_channel", json!({"channel":{"id":cid(1),"displayName":"新群名","props":null,"purpose":null,"picture":null}})).await;
    let after_patch = rows(&db, "channel").await.remove(0);
    assert_eq!(value(&after_patch, "props"), value(&before, "props"));
    for event in events
        .iter()
        .filter(|event| event["event"] == "im:channel:update")
    {
        for field in ["props", "purpose", "picture"] {
            assert!(event["data"].get(field).is_none());
        }
    }
}

/// 最后管理员取消、简介/昵称清空、完整空对象、false/0 都是真正有效写入。
#[tokio::test]
async fn explicit_clears_and_last_admin_demotion_reach_database_and_emit() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    deliver(&db, &mut h, "channel_member_update", initial()).await;
    let events = deliver(&db, &mut h, "channel_member_update", json!({"id":cid(1),"members":[{"id":"manager","role":"MEMBER","nickName":""}],"adminUsers":[]})).await;
    let event = events
        .iter()
        .find(|event| event["event"] == "im:channel:member-updated")
        .unwrap();
    assert_eq!(event["data"]["channel"]["adminUsers"], json!([]), "{event}");
    deliver(&db, &mut h, "update_channel", json!({"channel":{"id":cid(1),"purpose":"","orient":"","props":{},"picture":{},"channelIsTop":false,"mentionCount":0}})).await;
    deliver(
        &db,
        &mut h,
        "update_channel_member_nickName",
        json!({"channelId":cid(1),"userId":"owner","nickName":""}),
    )
    .await;
    let channel = rows(&db, "channel").await.remove(0);
    for field in ["purpose", "orient"] {
        assert_eq!(value(&channel, field), json!(""));
    }
    for field in ["props", "picture"] {
        assert_eq!(value(&channel, field), json!("{}"));
    }
    for field in ["is_top", "mention_count"] {
        assert_eq!(value(&channel, field), json!(0));
    }
    for member in rows(&db, "channel_member").await {
        if value(&member, "user_id") == json!("owner")
            || value(&member, "user_id") == json!("manager")
        {
            assert_eq!(value(&member, "nick_name"), json!(""));
        }
    }
}

/// 明确取消负责人仍需生效；members 仅带 id 时不能把旧 BOSS 留在库中。
#[tokio::test]
async fn explicit_boss_leave_demotes_existing_member_without_losing_nickname() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    deliver(&db, &mut h, "channel_member_update", initial()).await;
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        json!({"id":cid(1),"members":[{"id":"manager","role":"BOSS"}]}),
    )
    .await;
    deliver(&db, &mut h, "channel_member_update", json!({"id":cid(1),"members":[{"id":"manager"}],"boss":[{"id":"manager","option":"leave"}]})).await;
    let members = rows(&db, "channel_member").await;
    let member = members
        .iter()
        .find(|row| value(row, "user_id") == json!("manager"))
        .unwrap();
    assert_eq!(value(member, "role"), json!("MEMBER"));
    assert_eq!(value(member, "nick_name"), json!("管理员昵称"));
}

/// 缺少频道类型的稀疏消息仅更新已有行，不得建立默认空频道。
#[tokio::test]
async fn sparse_unknown_channel_does_not_insert_default_channel() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[], "owner", "team-a", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        json!({"id":cid(1),"displayName":"只有部分字段"}),
    )
    .await;
    assert!(rows(&db, "channel").await.is_empty());
}

/// 增量入口使用合法 notify 别名时不能把已提供值编译成空字符串。
#[tokio::test]
async fn increment_notify_alias_retains_the_supplied_value() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-a", "", "", "");
    deliver(&db, &mut h, "channel_member_update", initial()).await;
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        json!({"id":cid(1),"notify":"IGNORE"}),
    )
    .await;
    let channel = rows(&db, "channel").await.remove(0);
    assert_eq!(value(&channel, "notify_props"), json!("IGNORE"));
}
