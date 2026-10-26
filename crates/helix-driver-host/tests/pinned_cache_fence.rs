//! 置顶缓存回归：公开Tick产生写集，真实SQLite手动反转不同队列的执行顺序。
#[path = "../../helix-im/tests/common.rs"]
mod common;
use common::*;
use helix_core::effect::{ScanSpec, StorageOp};
use helix_core::ports::Storage;
use helix_core::tick::{AppCommand, InboundBytes, PortOutcome};
use helix_core::{Correlation, Effect, Tick};
use helix_driver_host::{execute_storage_ops, execute_storage_ops_atomic, HostStorage};
use serde_json::{json, Value};

/// 使用独立文件库和产品schema，退出时只删除本例文件。
async fn db(path: &str) -> HostStorage {
    let db = HostStorage::open_sqlite_url(path).await.unwrap();
    for sql in helix_im::IM_SCHEMA {
        db.execute_raw(sql).await.unwrap();
    }
    db.execute_raw("INSERT OR IGNORE INTO channel(id,type,top_count) VALUES ('chfixx00000000000000000001','O',1)").await.unwrap();
    db
}
/// 发起真实读命令，接受后端旧列表；只执行前置读取，截获待提交缓存事务。
async fn pending_write(db: &HostStorage, h: &mut Harness) -> (Correlation, Vec<StorageOp>) {
    h.step(
        Tick::Command(AppCommand::new(
            "im_channel_load_post_pinned",
            serde_json::to_vec(&json!({"channel_id":cid(1),"req_id":"old"})).unwrap(),
        )),
        1,
    );
    let http = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(corr.raw())
            } else {
                None
            }
        })
        .unwrap();
    h.step(
        port_reply_with_body(
            http,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"SUCCESS","data":[{"post":{"id":"old-pin"}}]}"#,
            )),
        ),
        2,
    );
    for _ in 0..4 {
        if let Some((corr, ops)) = h.sink.as_slice().iter().find_map(|e| {
            if let Effect::PersistAtomic { corr, ops } = e {
                Some((*corr, ops.clone()))
            } else {
                None
            }
        }) {
            return (corr, ops);
        }
        let (corr, ops) = h
            .sink
            .as_slice()
            .iter()
            .find_map(|e| {
                if let Effect::Persist { corr, ops } = e {
                    Some((*corr, ops.clone()))
                } else {
                    None
                }
            })
            .expect("fence read");
        h.step(
            Tick::PortReply {
                corr,
                outcome: execute_storage_ops(db, &ops).await,
            },
            3,
        );
    }
    panic!("no cache commit")
}
/// 在线删除与离线恢复共享的失效写集；按实际Effect的事务属性执行。
async fn invalidate(db: &HostStorage, h: &mut Harness) {
    h.step(Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({"action":"post_pin","data":{"channelId":cid(1),"postId":"old-pin","operation":"remove"}})).unwrap().into())),10);
    for e in h.sink.as_slice() {
        let outcome = match e {
            Effect::PersistAtomic { ops, .. } => execute_storage_ops_atomic(db, ops).await,
            Effect::PersistFire { ops } => execute_storage_ops(db, ops).await,
            _ => continue,
        };
        assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
    }
}
/// 独立SQL读取，不使用被测缓存转换函数。
async fn cached(db: &HostStorage) -> usize {
    db.scan(ScanSpec {
        table: "channel_pinned_projection",
        limit: None,
        filter: None,
        order_by: &[],
    })
    .await
    .unwrap()
    .len()
}
/// 两个不同table-set队列反向完成后，旧写必须回滚，重开后也不能恢复旧列表。
#[tokio::test]
async fn reversed_write_is_rejected_and_reopen_stays_empty() {
    let path =
        std::env::temp_dir().join(format!("helix-pin-reverse-{}.sqlite", std::process::id()));
    let url = path.to_str().unwrap();
    let db1 = db(url).await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (corr, ops) = pending_write(&db1, &mut h).await;
    invalidate(&db1, &mut h).await;
    let outcome = execute_storage_ops_atomic(&db1, &ops).await;
    assert!(
        matches!(outcome, PortOutcome::Err(_)),
        "old cache transaction must fail CAS: {outcome:?}"
    );
    h.step(Tick::PortReply { corr, outcome }, 11);
    let values: Vec<Value> = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|e| {
            if let Effect::Emit { event } = e {
                serde_json::from_slice(&event.0).ok()
            } else {
                None
            }
        })
        .collect();
    assert!(values.iter().any(|v| v["data"]["error"].is_string()));
    assert_eq!(cached(&db1).await, 0);
    drop(db1);
    let db2 = db(url).await;
    assert_eq!(cached(&db2).await, 0);
    let replay = execute_storage_ops_atomic(&db2, &ops).await;
    assert!(
        matches!(replay, PortOutcome::Err(_)),
        "fence must survive reopen"
    );
    drop(db2);
    std::fs::remove_file(path).unwrap();
}
/// 事务后段故障不得留下缓存或消耗CAS版本；原事务重试应可成功。
#[tokio::test]
async fn cache_transaction_failure_rolls_back_fence_and_body() {
    let path =
        std::env::temp_dir().join(format!("helix-pin-rollback-{}.sqlite", std::process::id()));
    let db1 = db(path.to_str().unwrap()).await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (_, ops) = pending_write(&db1, &mut h).await;
    let mut broken = ops.clone();
    broken.push(StorageOp::BatchUpdate(
        helix_core::effect::BatchUpdateSpec {
            table: "missing_pin_test_table",
            key_col: "id",
            key_vals: vec![helix_core::effect::SqlValue::Integer(1)],
            patch: vec![("x".into(), helix_core::effect::SqlValue::Integer(1))],
        },
    ));
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &broken).await,
        PortOutcome::Err(_)
    ));
    assert_eq!(cached(&db1).await, 0);
    let fence = db1
        .scan(ScanSpec {
            table: "channel_pinned_fence",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap();
    assert!(
        fence.is_empty(),
        "failed cache write must roll back fence creation"
    );

    assert!(matches!(
        execute_storage_ops_atomic(&db1, &ops).await,
        PortOutcome::Ok(_)
    ));
    assert_eq!(cached(&db1).await, 1);
    drop(db1);
    std::fs::remove_file(path).unwrap();
}

/// 截获公开TooLong恢复的原子写集，包含真实cursor提交，不手写失效SQL。
fn sync_invalidation(h: &mut Harness) -> Vec<StorageOp> {
    h.step(inbound_event(1, 99, 1), 20);
    let http = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(corr.raw())
            } else {
                None
            }
        })
        .unwrap();
    h.step(
        port_reply_with_body(http, enveloped(sync_reply_too_long(1, 100))),
        21,
    );
    let reload = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(corr.raw())
            } else {
                None
            }
        })
        .unwrap();
    h.step(
        port_reply_with_body(
            reload,
            enveloped(bytes::Bytes::from_static(
                br#"{"status":"SUCCESS","data":[]}"#,
            )),
        ),
        22,
    );
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::PersistAtomic { ops, .. } = e {
                Some(ops.clone())
            } else {
                None
            }
        })
        .unwrap()
}
/// 恢复事务与列表事务的队列key不同；恢复先提交后旧列表写入必须整笔拒绝。
#[tokio::test]
async fn offline_sync_reverse_queue_order_rejects_old_write() {
    let path = std::env::temp_dir().join(format!(
        "helix-pin-sync-reverse-{}.sqlite",
        std::process::id()
    ));
    let db1 = db(path.to_str().unwrap()).await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (_, old) = pending_write(&db1, &mut h).await;
    let sync = sync_invalidation(&mut h);
    assert_ne!(
        helix_driver_host::table_set_key(&old),
        helix_driver_host::table_set_key(&sync)
    );
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &sync).await,
        PortOutcome::Ok(_)
    ));
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &old).await,
        PortOutcome::Err(_)
    ));
    assert_eq!(cached(&db1).await, 0);
    drop(db1);
    std::fs::remove_file(path).unwrap();
}
/// 失效事务故障时，旧缓存/版本/cursor必须一起回滚；重试再原子删除。
#[tokio::test]
async fn invalidation_failure_rolls_back_cache_and_cursor() {
    let path = std::env::temp_dir().join(format!(
        "helix-pin-invalidate-rollback-{}.sqlite",
        std::process::id()
    ));
    let db1 = db(path.to_str().unwrap()).await;
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let (_, old) = pending_write(&db1, &mut h).await;
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &old).await,
        PortOutcome::Ok(_)
    ));
    let fence_before = db1
        .scan(ScanSpec {
            table: "channel_pinned_fence",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap();
    let sync = sync_invalidation(&mut h);
    let mut broken = sync.clone();
    broken.push(StorageOp::BatchUpdate(
        helix_core::effect::BatchUpdateSpec {
            table: "missing_pin_invalidation_table",
            key_col: "id",
            key_vals: vec![helix_core::effect::SqlValue::Integer(1)],
            patch: vec![("x".into(), helix_core::effect::SqlValue::Integer(1))],
        },
    ));
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &broken).await,
        PortOutcome::Err(_)
    ));
    assert_eq!(cached(&db1).await, 1);
    let fence_after = db1
        .scan(ScanSpec {
            table: "channel_pinned_fence",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap();
    assert_eq!(
        helix_core::port_codec::rows_to_reply_bytes(&fence_before),
        helix_core::port_codec::rows_to_reply_bytes(&fence_after),
        "failed invalidation must roll back revision increment"
    );

    let cursors = db1
        .scan(ScanSpec {
            table: "channel_event_cursor",
            limit: None,
            filter: None,
            order_by: &[],
        })
        .await
        .unwrap();
    assert!(
        cursors.is_empty(),
        "failed invalidation cannot advance recovery cursor"
    );
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &sync).await,
        PortOutcome::Ok(_)
    ));
    assert_eq!(cached(&db1).await, 0);
    assert!(matches!(
        execute_storage_ops_atomic(&db1, &old).await,
        PortOutcome::Err(_)
    ));
    drop(db1);
    std::fs::remove_file(path).unwrap();
}
