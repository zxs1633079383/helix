//! 公开Tick→真实SQLite回归：旧共享快照不覆盖新更新，事务与回调均受守卫。
#[path = "../../helix-im/tests/common.rs"]
mod common;
use common::*;
use helix_core::effect::{GetSpec, SqlValue};
use helix_core::ports::Storage;
use helix_core::tick::{InboundBytes, PortOutcome};
use helix_core::{Effect, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use serde_json::json;
/// 公开increment写集延迟至在线新名称持久化之后执行。
#[tokio::test]
async fn old_increment_must_not_overwrite_new_channel_name() {
    let db = HostStorage::open_sqlite_url("file:channel_scalar_probe?mode=memory&cache=shared")
        .await
        .unwrap();
    for sql in helix_im::IM_SCHEMA {
        db.execute_raw(sql).await.unwrap();
    }
    db.execute_raw("INSERT INTO channel(id,type,display_name) VALUES ('chfixx00000000000000000001','O','initial')").await.unwrap();
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"increment_channel","data":{"id":cid(1),"type":"O","displayName":"old","updateAt":100,"lastEventSeq":0}})).unwrap().into())),1);
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let old = h
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
        .expect("old snapshot batch");
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"update_channel","data":{"channel":{"id":cid(1),"displayName":"new","updateAt":200}}})).unwrap().into())),2);
    let mut applied = false;
    for effect in h.sink.as_slice() {
        let result = match effect {
            Effect::PersistFire { ops } | Effect::Persist { ops, .. } => {
                execute_storage_ops(&db, ops).await
            }
            Effect::PersistAtomic { ops, .. } => execute_storage_ops_atomic(&db, ops).await,
            _ => continue,
        };
        assert!(matches!(result, PortOutcome::Ok(_)), "{result:?}");
        applied = true;
    }
    assert!(applied, "online update must produce writes");
    let key = GetSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text(cid(1)),
    };
    let before = db.get(key.clone()).await.unwrap().unwrap();
    assert!(
        before
            .iter()
            .any(|(k, v)| k == "display_name" && matches!(v,SqlValue::Text(s) if s=="new")),
        "new online name must really be stored"
    );
    assert!(matches!(
        execute_storage_ops_atomic(&db, &old).await,
        PortOutcome::Ok(_)
    ));
    let after = db.get(key).await.unwrap().unwrap();
    assert!(
        after
            .iter()
            .any(|(k, v)| k == "display_name" && matches!(v,SqlValue::Text(s) if s=="new")),
        "old snapshot overwrote newer name: {after:?}"
    );
}

use helix_core::effect::{BatchUpdateSpec, StorageOp};
use helix_core::Correlation;
use serde_json::Value;

/// 独立磁盘库同时覆盖产品schema与重开；只清理当前测试拥有的文件。
struct TestDb {
    path: String,
    db: Option<HostStorage>,
}
impl TestDb {
    /// 每例独立建库并seed真实账号归属。
    async fn new(name: &str) -> Self {
        let path = std::env::temp_dir()
            .join(format!(
                "helix-channel-{}-{name}.sqlite",
                std::process::id()
            ))
            .to_string_lossy()
            .into_owned();
        let _ = std::fs::remove_file(&path);
        let db = HostStorage::open_sqlite_url(&path).await.unwrap();
        for sql in helix_im::IM_SCHEMA {
            db.execute_raw(sql).await.unwrap();
        }
        db.execute_raw("INSERT INTO channel(id,type,display_name,purpose,user_id,team_id) VALUES ('chfixx00000000000000000001','O','initial','keep','viewer-444','tenant-1')").await.unwrap();
        Self { path, db: Some(db) }
    }
    /// 返回当前打开的测试连接。
    fn storage(&self) -> &HostStorage {
        self.db.as_ref().unwrap()
    }
    /// 丢弃连接后重新打开同一个磁盘库。
    async fn reopen(&mut self) {
        drop(self.db.take());
        self.db = Some(HostStorage::open_sqlite_url(&self.path).await.unwrap());
    }
}
impl Drop for TestDb {
    fn drop(&mut self) {
        drop(self.db.take());
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.path));
        }
    }
}
/// 公开WS入口产生待持久化写集，测试不手造业务写入SQL。
fn update(h: &mut Harness, mut data: Value) -> (Correlation, Vec<StorageOp>) {
    data["id"] = json!(cid(1));
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action":"update_channel","data":{"channel":data}}))
                .unwrap()
                .into(),
        )),
        50,
    );
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("atomic update")
}
/// 保留离线批次，不执行，模拟它在另一table-set队列迟到。
fn snapshot(h: &mut Harness, mut data: Value) -> Vec<StorageOp> {
    data["id"] = json!(cid(1));
    data["type"] = json!("O");
    h.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action":"increment_channel","data":data}))
                .unwrap()
                .into(),
        )),
        10,
    );
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .expect("increment atomic")
}
/// 独立读取原始落库列，避免用被测投影转换器替自己作证。
async fn stored(db: &HostStorage) -> Value {
    let row = db
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(cid(1)),
        })
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice::<Value>(&helix_core::port_codec::rows_to_reply_bytes(&[row])).unwrap()
        [0]
    .clone()
}
/// 完成write→readback生命周期；回传真实存储结果，直到领域事件出口。
async fn finish(db: &HostStorage, h: &mut Harness, corr: Correlation, outcome: PortOutcome) {
    h.step(Tick::PortReply { corr, outcome }, 100);
    if let Some((corr, ops)) = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
        _ => None,
    }) {
        let outcome = execute_storage_ops(db, &ops).await;
        h.step(Tick::PortReply { corr, outcome }, 101);
    }
}
/// 只收集由公开生命周期发布的频道更新事件。
fn updates(h: &Harness) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|v| v["event"] == "im:channel:update")
        .collect()
}

/// 整组共享字段遭遇不同table-set逆序后不回退，重开继续拒旧。
#[tokio::test]
async fn shared_fields_reverse_queues_and_reopen() {
    let mut t = TestDb::new("shared").await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let old = snapshot(
        &mut h,
        json!({"displayName":"old","purpose":"old","header":"old","picture":{"url":"old"},"props":{"old":true},"source":{"id":"old"},"orient":"old","mentionPermission":"ALL","noticePermission":"ALL","topPermission":"ALL","adminMaxCount":99,"isActive":true,"deleteAt":99,"createAt":10,"teamId":"old","rootId":"old-root","rootPostId":"old-post","pictureType":"old","createBy":"old","updateBy":"old","targetUsers":["old"],"updateAt":100}),
    );
    let (corr, new) = update(
        &mut h,
        json!({"displayName":"new","purpose":"","header":"new","picture":{"url":"new"},"props":{},"source":{},"orient":"new","mentionPermission":"ADMIN","noticePermission":"ADMIN","topPermission":"ADMIN","adminMaxCount":0,"isActive":false,"deleteAt":0,"createAt":20,"teamId":"new","type":"P","rootId":"new-root","rootPostId":"new-post","pictureType":"new","createBy":"new","updateBy":"new","targetUsers":["new"],"updateAt":200}),
    );
    assert_ne!(
        helix_driver_host::ops::table_set_key(&old),
        helix_driver_host::ops::table_set_key(&new)
    );
    let outcome = execute_storage_ops_atomic(t.storage(), &new).await;
    assert!(matches!(outcome, PortOutcome::Ok(_)));
    finish(t.storage(), &mut h, corr, outcome).await;
    assert_eq!(updates(&h)[0]["data"]["displayName"], "new");
    let before = stored(t.storage()).await;
    assert!(matches!(
        execute_storage_ops_atomic(t.storage(), &old).await,
        PortOutcome::Ok(_)
    ));
    t.reopen().await;
    assert!(matches!(
        execute_storage_ops_atomic(t.storage(), &old).await,
        PortOutcome::Ok(_)
    ));
    let after = stored(t.storage()).await;
    for key in [
        "display_name",
        "purpose",
        "header",
        "picture",
        "props",
        "source",
        "orient",
        "mention_permission",
        "notice_permission",
        "top_permission",
        "admin_max_count",
        "is_active",
        "delete_at",
        "created_at",
        "team_id",
        "type",
        "root_id",
        "root_post_id",
        "picture_type",
        "create_by",
        "update_by",
        "target_users",
        "snapshot_updated_at",
    ] {
        assert_eq!(after[key], before[key], "{key}");
    }
    assert_eq!(after["purpose"], "");
    assert_eq!(after["is_active"], 0);
    assert_eq!(after["admin_max_count"], 0);
}

/// 稀疏缺省保留、同版本补丁可合并，旧/无版本共享更新不绕过水位；独立计数仍可前进。
#[tokio::test]
async fn sparse_equal_legacy_and_independent_fields() {
    let t = TestDb::new("sparse").await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    for payload in [
        json!({"displayName":"new","updateAt":200}),
        json!({"header":"equal","updateAt":200}),
    ] {
        let (_, ops) = update(&mut h, payload);
        assert!(matches!(
            execute_storage_ops_atomic(t.storage(), &ops).await,
            PortOutcome::Ok(_)
        ));
    }
    for payload in [
        json!({"displayName":"old","updateAt":100}),
        json!({"displayName":"legacy","unreadCount":3}),
    ] {
        let (corr, ops) = update(&mut h, payload);
        let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
        finish(t.storage(), &mut h, corr, outcome).await;
        assert!(
            updates(&h).is_empty(),
            "rejected input must not emit success"
        );
    }
    let row = stored(t.storage()).await;
    assert_eq!(row["display_name"], "new");
    assert_eq!(row["purpose"], "keep");
    assert_eq!(row["header"], "equal");
    assert_eq!(row["unread_count"], 3);
}

/// 后段失败回滚属性、水位和同批其他写；重开后合法重试不能被假水位挡住。
#[tokio::test]
async fn rollback_restores_values_and_version_after_reopen() {
    let mut t = TestDb::new("rollback").await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (corr, mut ops) = update(
        &mut h,
        json!({"displayName":"failed","updateAt":300,"unreadCount":8}),
    );
    ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "missing_channel_failure_table",
        key_col: "id",
        key_vals: vec![SqlValue::Text(cid(1))],
        patch: vec![("x".into(), SqlValue::Integer(1))],
    }));
    let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
    assert!(matches!(outcome, PortOutcome::Err(_)));
    finish(t.storage(), &mut h, corr, outcome).await;
    assert!(updates(&h).is_empty());
    t.reopen().await;
    let row = stored(t.storage()).await;
    assert_eq!(row["display_name"], "initial");
    assert_eq!(row["snapshot_updated_at"], 0);
    assert_eq!(row["unread_count"], 0);
    let mut fresh = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (corr, ops) = update(&mut fresh, json!({"displayName":"retry","updateAt":200}));
    let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
    finish(t.storage(), &mut fresh, corr, outcome).await;
    assert_eq!(updates(&fresh)[0]["data"]["displayName"], "retry");
}

/// 较早读结果已算出但回调迟到；新更新到达后不能再发布旧成功。
#[tokio::test]
async fn delayed_readback_cannot_emit_old_success() {
    for revision in [100, 200] {
        let t = TestDb::new(&format!("late-{revision}")).await;
        let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
        let (corr, ops) = update(&mut h, json!({"displayName":"old","updateAt":100}));
        let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
        h.step(Tick::PortReply { corr, outcome }, 70);
        let (read, ops) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
                _ => None,
            })
            .unwrap();
        let delayed = execute_storage_ops(t.storage(), &ops).await;
        let (corr, ops) = update(&mut h, json!({"displayName":"new","updateAt":revision}));
        let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
        finish(t.storage(), &mut h, corr, outcome).await;
        assert_eq!(updates(&h)[0]["data"]["displayName"], "new");
        h.step(
            Tick::PortReply {
                corr: read,
                outcome: delayed,
            },
            200,
        );
        assert!(updates(&h).is_empty());
    }
}

/// 旧库本地updated_at可大于服务端时间；迁移水位从0开始，不把本地触发器时间误作权威。
#[tokio::test]
async fn migration_and_untyped_snapshot_preserve_contracts() {
    let mut t = TestDb::new("migration").await;
    t.storage()
        .execute_raw("ALTER TABLE channel DROP COLUMN snapshot_updated_at")
        .await
        .unwrap();
    t.storage()
        .execute_raw("UPDATE channel SET updated_at=9000000000000")
        .await
        .unwrap();
    let migration = helix_im::IM_SCHEMA_MIGRATIONS
        .iter()
        .find(|sql| sql.contains("ADD COLUMN snapshot_updated_at"))
        .unwrap();
    t.storage().execute_raw(migration).await.unwrap();
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (corr, ops) = update(&mut h, json!({"displayName":"server","updateAt":200}));
    let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
    finish(t.storage(), &mut h, corr, outcome).await;
    assert_eq!(updates(&h)[0]["data"]["displayName"], "server");
    t.reopen().await;
    let (corr, ops) = update(&mut h, json!({"displayName":"old","updateAt":100}));
    // 非atomic执行器也必须兑现相同版本谓词。
    let outcome = execute_storage_ops(t.storage(), &ops).await;
    finish(t.storage(), &mut h, corr, outcome).await;
    assert!(updates(&h).is_empty());
    assert_eq!(stored(t.storage()).await["display_name"], "server");
    t.storage()
        .execute_raw("DELETE FROM channel")
        .await
        .unwrap();
    let (corr, ops) = update(&mut h, json!({"displayName":"phantom","updateAt":300}));
    let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
    finish(t.storage(), &mut h, corr, outcome).await;
    assert!(updates(&h).is_empty());
    assert!(t
        .storage()
        .get(GetSpec {
            table: "channel",
            key_col: "id",
            key_val: SqlValue::Text(cid(1))
        })
        .await
        .unwrap()
        .is_none());
}

/// 较新离线快照先落库，旧在线PATCH随后执行，同样不得回退或发布旧成功。
#[tokio::test]
async fn old_online_patch_after_new_snapshot_is_rejected() {
    let t = TestDb::new("old-online").await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (corr, old) = update(
        &mut h,
        json!({"displayName":"old","purpose":"old","updateAt":100}),
    );
    let newer = snapshot(
        &mut h,
        json!({"displayName":"snapshot","purpose":"new","updateAt":200}),
    );
    assert!(matches!(
        execute_storage_ops_atomic(t.storage(), &newer).await,
        PortOutcome::Ok(_)
    ));
    let outcome = execute_storage_ops_atomic(t.storage(), &old).await;
    finish(t.storage(), &mut h, corr, outcome).await;
    assert!(updates(&h).is_empty());
    assert_eq!(stored(t.storage()).await["display_name"], "snapshot");
    h.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(bytes::Bytes::new())),
        },
        500,
    );
    assert!(updates(&h).is_empty(), "duplicate completion stays silent");
}

/// HTTP设置回包与WS也写同一共享行；迟到HTTP不能绕过离线/在线版本守卫。
#[tokio::test]
async fn stale_http_settings_cannot_overwrite_new_ws_snapshot() {
    let t = TestDb::new("http-settings").await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    h.step(
        command_json(
            "im_channel_change_info",
            json!({"channel_id":cid(1),"display_name":"old"}),
        ),
        1,
    );
    let http = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    let (corr, ops) = update(
        &mut h,
        json!({"displayName":"new","purpose":"new","header":"new","updateAt":200}),
    );
    let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
    finish(t.storage(), &mut h, corr, outcome).await;
    h.step(port_reply_with_body(http,enveloped(serde_json::to_vec(&json!({"status":"SUCCESS","data":{"id":cid(1),"displayName":"old","purpose":"old","header":"old","settingVersion":100}})).unwrap().into())),200);
    let pending = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops } => {
                Some((*corr, ops.clone()))
            }
            _ => None,
        })
        .unwrap();
    let outcome = execute_storage_ops_atomic(t.storage(), &pending.1).await;
    finish(t.storage(), &mut h, pending.0, outcome).await;
    assert_eq!(stored(t.storage()).await["display_name"], "new");
    assert!(updates(&h).is_empty());
}

/// HTTP有效权威必须经历写和读两个屏障；失败与重复回执均不发布群属性成功。
#[tokio::test]
async fn http_settings_success_and_failure_lifecycle() {
    for fail_write in [false, true] {
        let t = TestDb::new(&format!("http-life-{fail_write}")).await;
        let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
        h.step(
            command_json(
                "im_channel_change_info",
                json!({"channel_id":cid(1),"display_name":"current","req_id":"settings-life"}),
            ),
            1,
        );
        let http = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::Http { corr, .. } => Some(corr.raw()),
                _ => None,
            })
            .unwrap();
        h.step(port_reply_with_body(http,enveloped(serde_json::to_vec(&json!({"status":"SUCCESS","data":{"id":cid(1),"displayName":"current","purpose":"","header":"new","picture":{"url":"current"},"mentionPermission":"ADMIN","settingVersion":300}})).unwrap().into())),200);
        assert!(updates(&h).is_empty());
        let (corr, mut ops) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| match e {
                Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
                _ => None,
            })
            .unwrap();
        if fail_write {
            ops.push(StorageOp::BatchUpdate(BatchUpdateSpec {
                table: "missing_settings_table",
                key_col: "id",
                key_vals: vec![SqlValue::Text(cid(1))],
                patch: vec![("x".into(), SqlValue::Integer(1))],
            }));
        }
        let outcome = execute_storage_ops_atomic(t.storage(), &ops).await;
        finish(t.storage(), &mut h, corr, outcome).await;
        let row = stored(t.storage()).await;
        if fail_write {
            assert!(updates(&h).is_empty());
            assert_eq!(row["display_name"], "initial");
            assert_eq!(row["snapshot_updated_at"], 0);
        } else {
            assert_eq!(updates(&h)[0]["data"]["displayName"], "current");
            assert_eq!(row["snapshot_updated_at"], 300);
            assert_eq!(row["picture"], "{\"url\":\"current\"}");
            assert_eq!(row["mention_permission"], "ADMIN");
        }
        h.step(port_reply_ok(corr.raw()), 300);
        assert!(updates(&h).is_empty());
    }
}
