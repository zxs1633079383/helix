//! 接龙异常 evt 空白行迁移：真实 SQLite 验证严格匹配、保留边界与幂等。
use rusqlite::Connection;

/// 只有无服务端身份、无内容且键与频道/序号完全一致的历史空白行可以删除。
#[test]
fn chain_placeholder_cleanup_preserves_real_and_pending_messages() {
    let db = Connection::open_in_memory().unwrap();
    for sql in helix_im::IM_SCHEMA {
        db.execute_batch(sql).unwrap();
    }
    for (key, id, body, author, seq, created) in [
        ("evt:channel:291", "", "", "", 291, 0),
        ("evt:channel:320", "", "", "", 320, 0),
        ("evt:channel:321", "real-post", "", "", 321, 0),
        ("evt:channel:322", "", "正文", "", 322, 0),
        ("evt:channel:323", "", "", "author", 323, 0),
        ("evt:channel:324", "", "", "", 324, 1),
        ("evt:other:325", "", "", "", 325, 0),
        ("helix_tmp_pending", "", "", "", 326, 0),
    ] {
        db.execute("INSERT INTO message(temporary_id,id,channel_id,message,user_id,event_seq,create_at,type,send_status) VALUES(?1,?2,'channel',?3,?4,?5,?6,'TEXT','sent')", rusqlite::params![key,id,body,author,seq,created]).unwrap();
    }
    for _ in 0..2 {
        for sql in helix_im::IM_SCHEMA_MIGRATIONS {
            if let Err(error) = db.execute_batch(sql) {
                assert!(
                    error.to_string().contains("duplicate column name"),
                    "{error}"
                );
            }
        }
        let remaining: Vec<String> = db
            .prepare("SELECT temporary_id FROM message ORDER BY temporary_id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            remaining,
            vec![
                "evt:channel:321",
                "evt:channel:322",
                "evt:channel:323",
                "evt:channel:324",
                "evt:other:325",
                "helix_tmp_pending"
            ]
        );
    }
}

#[path = "../../helix-im/tests/common.rs"]
mod common;
use common::{cid, port_reply_ok, sid, EffectKind, Harness};
use helix_core::effect::ScanSpec;
use helix_core::ports::Storage;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Effect, Tick};
use helix_driver_host::{execute_storage_ops_atomic, HostStorage};
use serde_json::{json, Value};

/// 从产品 Effect 中借用原子写集，不复制或手工拼接存储操作。
fn pending_write(h: &Harness) -> (helix_core::Correlation, &[helix_core::effect::StorageOp]) {
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| {
            if let Effect::PersistAtomic { corr, ops } = e {
                Some((*corr, ops.as_slice()))
            } else {
                None
            }
        })
        .unwrap()
}

/// 不同到达顺序都必须由两个持久屏障独立完成 chain 投影和完整 message。
#[tokio::test]
async fn chain_live_dual_events_commit_real_sqlite_without_placeholder() {
    for dedicated_first in [false, true] {
        let db = HostStorage::open_sqlite_url(":memory:").await.unwrap();
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
        let mut h = Harness::new_with_identity(&[1], "viewer", "tenant", "", "", "");
        let chain_data = json!({"eventId":"chain-event-1", "eventSeq":1, "action":"im:post_chain:publish",
            "channelId":cid(1), "chainSummary":{"chainId":"chain-one", "channelId":cid(1),
                "mode":"TEXT", "status":"PUBLISHED", "title":"吃饭啦", "revision":2,
                "declarationPostId":sid(1), "creatorId":"author", "teamId":"tenant"}});
        let dedicated = Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({
                "action":"im:post_chain:publish", "event_seq":1, "data":chain_data
            }))
            .unwrap()
            .into(),
        ));
        let canonical = Tick::Inbound(InboundBytes(serde_json::to_vec(&json!({
            "action":"channel_stream_event", "data":{"channel_id":cid(1), "stream_seq":1,
                "event_id":"chain-event-1", "event_type":1, "actor_id":"author", "msg_id":sid(1),
                "redacted":false, "payload":{"action":"im:post_chain:publish", "chain_summary":chain_data["chainSummary"]}}
        })).unwrap().into()));
        let mut domain_events = Vec::<Value>::new();
        if dedicated_first {
            h.step(dedicated.clone(), 1000);
            let (corr, ops) = pending_write(&h);
            let outcome = execute_storage_ops_atomic(&db, ops).await;
            assert!(matches!(&outcome, PortOutcome::Ok(_)), "{outcome:?}");
            h.step(Tick::PortReply { corr, outcome }, 1001);
            domain_events.extend(h.sink.as_slice().iter().filter_map(|e| {
                if let Effect::Emit { event } = e {
                    Some(serde_json::from_slice(&event.0).unwrap())
                } else {
                    None
                }
            }));
        }
        let effects = h.step(canonical, 1002);
        let corr = effects
            .iter()
            .find_map(|e| {
                if let EffectKind::Http(c) = e {
                    Some(*c)
                } else {
                    None
                }
            })
            .expect("完整消息必须经过 posts/get");
        if !dedicated_first {
            h.step(dedicated, 1003);
            let (chain_corr, ops) = pending_write(&h);
            let outcome = execute_storage_ops_atomic(&db, ops).await;
            assert!(matches!(&outcome, PortOutcome::Ok(_)), "{outcome:?}");
            h.step(
                Tick::PortReply {
                    corr: chain_corr,
                    outcome,
                },
                1004,
            );
            domain_events.extend(h.sink.as_slice().iter().filter_map(|e| {
                if let Effect::Emit { event } = e {
                    Some(serde_json::from_slice(&event.0).unwrap())
                } else {
                    None
                }
            }));
        }
        let body = json!({"status":"SUCCESS", "data":[{"id":sid(1), "channelId":cid(1),
            "userId":"author", "teamId":"tenant", "type":"TEXT_CHAIN", "message":"文字接龙：吃饭啦",
            "viewers":["all"], "createAt":100, "props":{"chain":{"chainId":"chain-one", "mode":"TEXT", "messageRole":"DECLARATION", "title":"吃饭啦"}}}]});
        h.step(
            Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr),
                outcome: PortOutcome::Ok(ReplyBytes(common::enveloped(
                    serde_json::to_vec(&body).unwrap().into(),
                ))),
            },
            1005,
        );
        let (write_corr, ops) = pending_write(&h);
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|e| matches!(e, Effect::Emit { .. })));
        let outcome = execute_storage_ops_atomic(&db, ops).await;
        assert!(matches!(&outcome, PortOutcome::Ok(_)), "{outcome:?}");
        h.step(
            Tick::PortReply {
                corr: write_corr,
                outcome,
            },
            1006,
        );
        domain_events.extend(h.sink.as_slice().iter().filter_map(|e| {
            if let Effect::Emit { event } = e {
                Some(serde_json::from_slice(&event.0).unwrap())
            } else {
                None
            }
        }));
        let rows = db
            .scan(ScanSpec {
                table: "message",
                limit: None,
                filter: None,
                order_by: &[],
            })
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        let row = &rows[0];
        assert!(row.iter().any(|(k, v)| *k == "id"
            && matches!(v, helix_core::effect::SqlValue::Text(value) if value == &sid(1))));
        assert!(row.iter().any(|(k, v)| *k == "message"
            && matches!(v, helix_core::effect::SqlValue::Text(value) if value == "文字接龙：吃饭啦")));
        assert!(!format!("{rows:?}").contains("evt:"));
        assert_eq!(
            domain_events
                .iter()
                .filter(|e| e["event"] == "im:post_chain:publish")
                .count(),
            1,
            "{domain_events:?}"
        );
        assert_eq!(
            domain_events
                .iter()
                .filter(|e| e["event"] == "im:post:received")
                .count(),
            1,
            "{domain_events:?}"
        );
        assert_eq!(h.module.cursor_for(common::channel_id(1)), Some(1));
        assert!(h.step(port_reply_ok(write_corr.raw()), 1007).is_empty());
    }
}
