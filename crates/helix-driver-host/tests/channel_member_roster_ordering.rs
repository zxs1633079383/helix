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
    settle(db, h).await
}

/// 兑现当前输入留下的存储效果，HTTP仍由用例提供明确响应。
async fn settle(db: &HostStorage, h: &mut Harness) -> Vec<Value> {
    let mut pending = std::collections::VecDeque::new();
    let mut events = Vec::new();
    for _ in 0..24 {
        for effect in h.sink.as_slice() {
            match effect {
                Effect::PersistAtomic { corr, ops } => {
                    let outcome = execute_storage_ops_atomic(db, ops).await;

                    pending.push_back(Tick::PortReply {
                        corr: *corr,
                        outcome,
                    });
                }
                Effect::Persist { corr, ops } => {
                    let outcome = execute_storage_ops(db, ops).await;

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
                    eprintln!("REPLAY_BOUNDARY pending HTTP effect left unexecuted")
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

/// 当前成员快照的真实Go wire形态；role摘要与join同源，版本由authority给出。
fn snapshot(version: i64, role: &str) -> Value {
    json!({"id":cid(1),"type":"P","teamId":"team-1","updateAt":version,
 "owner":{"id":"owner","teamId":"team-1"},
 "adminUsers":if role=="MANAGER"{json!([{"id":"manager","teamId":"team-1"}])}else{json!([])},
 "memberChange":{"join":[{"id":"owner","teamId":"team-1","role":"CREATOR"},{"id":"manager","teamId":"team-1","role":role,"nickName":"当前昵称"}]}})
}

/// 只从真实SQLite成员行断言，不复用被测的角色归一代码。
async fn assert_manager(db: &HostStorage, role: &str) {
    let roster = rows(db, "channel_member").await;
    let members: Vec<_> = roster
        .iter()
        .filter(|r| value(r, "user_id") == json!("manager"))
        .collect();
    assert_eq!(members.len(), 1, "duplicate/missing manager rows");
    assert_eq!(value(members[0], "role"), json!(role));
    assert_eq!(value(members[0], "nick_name"), json!("当前昵称"));
}

/// 同会话先新后旧/重复到达：旧角色不得持久化，也不发布旧成功。
#[tokio::test]
async fn newer_then_older_and_duplicate_preserve_member_role() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    let stale = deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(100, "MEMBER"),
    )
    .await;
    assert!(stale.is_empty());
    assert_manager(&db, "ADMIN").await;
    for _ in 0..3 {
        deliver(
            &db,
            &mut h,
            "channel_member_update",
            snapshot(200, "MANAGER"),
        )
        .await;
    }
    assert_manager(&db, "ADMIN").await;
}

/// 文件库关闭重开后内存缓存为空，仍由持久fence拒绝旧帧。
#[tokio::test]
async fn reopened_database_rejects_old_member_snapshot() {
    let path = std::env::temp_dir().join(format!(
        "helix-roster-reopen-{}-{}.sqlite",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let url = format!("sqlite://{}?mode=rwc", path.display());
    let db = HostStorage::open_sqlite_url(&url).await.unwrap();
    for sql in helix_im::IM_SCHEMA {
        db.execute_raw(sql).await.unwrap();
    }
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    drop(h);
    drop(db);
    let db = HostStorage::open_sqlite_url(&url).await.unwrap();
    let mut reopened = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    let stale = deliver(
        &db,
        &mut reopened,
        "channel_member_update",
        snapshot(100, "MEMBER"),
    )
    .await;
    assert!(stale.is_empty());
    assert_manager(&db, "ADMIN").await;
    drop(db);
    let _ = std::fs::remove_file(path);
}

/// 同频道两个写集反序执行，DB版本冲突必须回滚整批旧成员写。
#[tokio::test]
async fn reverse_storage_execution_does_not_regress_roster() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    let mut batches = Vec::new();
    for frame in [snapshot(100, "MEMBER"), snapshot(200, "MANAGER")] {
        h.step(
            Tick::Inbound(InboundBytes(
                serde_json::to_vec(&json!({"action":"channel_member_update","data":frame}))
                    .unwrap()
                    .into(),
            )),
            1,
        );
        let ops = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| {
                if let Effect::PersistAtomic { ops, .. } = e {
                    Some(ops.clone())
                } else {
                    None
                }
            })
            .unwrap();
        batches.push(ops);
    }
    assert!(matches!(
        execute_storage_ops_atomic(&db, &batches[1]).await,
        PortOutcome::Ok(_)
    ));
    assert!(matches!(
        execute_storage_ops_atomic(&db, &batches[0]).await,
        PortOutcome::Err(_)
    ));
    assert_manager(&db, "ADMIN").await;
}

/// 后续成员SQL失败时，先写的fence也必须回滚；失败不得阻断下一次有效恢复。
#[tokio::test]
async fn failed_member_transaction_rolls_back_version_fence() {
    use helix_core::effect::{BatchUpdateSpec, StorageOp};
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(100, "MANAGER"),
    )
    .await;
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(
                &json!({"action":"channel_member_update","data":snapshot(300,"MEMBER")}),
            )
            .unwrap()
            .into(),
        )),
        2,
    );
    let (corr, mut ops) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::PersistAtomic { corr, ops } = e {
                Some((*corr, ops.clone()))
            } else {
                None
            }
        })
        .unwrap();
    ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "missing_roster_probe_table",
        key_col: "id",
        key_vals: vec![SqlValue::Text(cid(1))],
        patch: vec![("role".into(), SqlValue::Text("MEMBER".into()))],
    }));
    let result = execute_storage_ops_atomic(&db, &ops).await;
    assert!(matches!(result, PortOutcome::Err(_)));
    h.step(
        Tick::PortReply {
            corr,
            outcome: result,
        },
        3,
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Emit { .. })));
    let fences = rows(&db, "channel_member_roster_fence").await;
    assert_eq!(value(&fences[0], "revision"), json!(100));
    assert_manager(&db, "ADMIN").await;
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    assert_manager(&db, "ADMIN").await;
}

/// canonical完整角色已版本化后，无版本legacy delta不能再次降级。
#[tokio::test]
async fn legacy_role_delta_cannot_override_versioned_roster() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    let stale = deliver(
        &db,
        &mut h,
        "channel_member_role_updated",
        json!({"channelId":cid(1),"userIds":["manager"],"role":"MEMBER"}),
    )
    .await;
    assert!(stale.is_empty());
    assert_manager(&db, "ADMIN").await;
}

/// 已有错误角色通过同版本的权威全量补水修正；重复交付仍只有一条成员。
#[tokio::test]
async fn authoritative_hydration_repairs_corrupt_role_at_same_version() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    db.execute_raw("UPDATE channel_member SET role='MEMBER' WHERE user_id='manager'")
        .await
        .unwrap();
    let mut fresh = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    for _ in 0..2 {
        deliver(
            &db,
            &mut fresh,
            "increment_channel",
            snapshot(200, "MANAGER"),
        )
        .await;
        deliver(&db, &mut fresh, "increment_channel_end", json!({})).await;
    }
    assert_manager(&db, "ADMIN").await;
}

/// 群主转让之后迟到的旧群主帧不能复活旧群主成员。
#[tokio::test]
async fn owner_transfer_survives_late_old_owner_snapshot() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "manager", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(100, "MANAGER"),
    )
    .await;
    let mut transferred = snapshot(200, "MANAGER");
    transferred["owner"] = json!({"id":"manager","teamId":"team-1"});
    transferred["adminUsers"] = json!([]);
    transferred["memberChange"] = json!({"join":[{"id":"manager","teamId":"team-1","role":"CREATOR","nickName":"当前昵称"}],"leave":[{"id":"owner"}]});
    deliver(&db, &mut h, "channel_member_update", transferred).await;
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(100, "MEMBER"),
    )
    .await;
    assert_manager(&db, "OWNER").await;
    let roster = rows(&db, "channel_member").await;
    assert!(!roster.iter().any(|r| value(r, "user_id") == json!("owner")));
    let channels = rows(&db, "channel").await;
    let owner: Value =
        serde_json::from_str(value(&channels[0], "owner").as_str().unwrap()).unwrap();
    assert_eq!(owner["id"], json!("manager"));
}

/// 一个冷缓存旧频道不能让同批新频道永久等待；冲突后重新读取权威分页。
#[tokio::test]
async fn stale_channel_in_global_batch_recovers_other_channel_via_authority() {
    let db = database().await;
    let mut first = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut first,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    let mut h = Harness::new_with_identity(&[1, 2], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "hello",
        json!({"connectionId":"recovery-connection"}),
    )
    .await;
    let mut other = snapshot(300, "MANAGER");
    other["id"] = json!(cid(2));
    deliver(&db, &mut h, "increment_channel", snapshot(100, "MEMBER")).await;
    deliver(&db, &mut h, "increment_channel", other.clone()).await;
    deliver(&db, &mut h, "increment_channel_end", json!({})).await;
    let corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channels/load/increment/page") => {
                Some(*corr)
            }
            _ => None,
        })
        .expect("stale mixed batch must request fresh authority instead of stalling");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(common::enveloped(
                serde_json::to_vec(
                    &json!({"status":"SUCCESS","data":{"manifest":[cid(1),cid(2)],
                "channels":[snapshot(200,"MANAGER"),other]}}),
                )
                .unwrap()
                .into(),
            ))),
        },
        1002,
    );
    settle(&db, &mut h).await;
    let channels = rows(&db, "channel").await;
    assert!(
        channels.iter().any(|row| value(row, "id") == json!(cid(2))),
        "fresh second channel must commit after authority refresh"
    );
    let fence = rows(&db, "channel_member_roster_fence").await;
    assert_eq!(fence.len(), 2);
    let roster = rows(&db, "channel_member").await;
    assert!(roster
        .iter()
        .filter(|row| value(row, "user_id") == json!("manager"))
        .all(|row| value(row, "role") == json!("ADMIN")));
}

/// 已有数据库按追加迁移建立版本表；重跑迁移不改既有成员行。
#[tokio::test]
async fn additive_fence_migration_preserves_existing_member_rows() {
    let db = database().await;
    let mut h = Harness::new_with_identity(&[1], "owner", "team-1", "", "", "");
    deliver(
        &db,
        &mut h,
        "channel_member_update",
        snapshot(200, "MANAGER"),
    )
    .await;
    db.execute_raw("DROP TABLE channel_member_roster_fence")
        .await
        .unwrap();
    for _ in 0..2 {
        for sql in helix_im::IM_SCHEMA_MIGRATIONS {
            if let Err(error) = db.execute_raw(sql).await {
                assert!(
                    error.to_string().contains("duplicate column name"),
                    "{error:?}"
                );
            }
        }
        assert_manager(&db, "ADMIN").await;
        assert!(rows(&db, "channel_member_roster_fence").await.is_empty());
    }
}
