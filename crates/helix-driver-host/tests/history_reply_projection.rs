//! 历史exact/initial定位：公开Tick→HTTP隔离回包→实际SQLite→读回→终态。
#[path = "../../helix-im/tests/common.rs"]
mod common;
use bytes::Bytes;
use helix_core::effect::{GetSpec, SqlValue};
use helix_core::ports::Storage;
use helix_core::tick::PortOutcome;
use helix_core::{Effect, Module, Tick};
use helix_driver_host::{execute_storage_ops, HostStorage};
use serde_json::{json, Value};

/// 两个公开导航入口都须保留无引用null，不能把扩展字段通用空对象视作引用。
#[tokio::test]
async fn navigation_preserves_nullable_reply_through_sqlite() {
    for initial in [false, true] {
        for reply in [
            Value::Null,
            json!(""),
            json!({"repliedUserId":"sender-2","message":"引用正文","viewers":["viewer-1"]}),
        ] {
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
            let mut h =
                common::Harness::new_with_identity(&[0x11f], "viewer-1", "tenant-1", "", "", "");
            let id = "srvfix00000000000000000011";
            let payload = if initial {
                json!({"req_id":"reply-regression","post_id":id,"page_size":2})
            } else {
                json!({"req_id":"reply-regression","post_ids":[id],"locate":true})
            };
            h.step(
                common::command_json(
                    if initial {
                        "im_get_posts_after_index"
                    } else {
                        "im_get_posts"
                    },
                    payload,
                ),
                1,
            );
            let mut events = Vec::<Value>::new();
            let mut readback = false;
            let mut http_count = 0;
            for now in 2..12 {
                if h.sink.is_empty() {
                    break;
                }
                let batch = std::mem::take(&mut h.sink);
                for fx in batch.as_slice() {
                    match fx {
                        Effect::Http { corr, req } => {
                            http_count += 1;
                            assert!(req.url.ends_with(if initial {
                                "/posts/getPostsAfterIndex"
                            } else {
                                "/posts/get"
                            }));
                            let post = json!({"id":id,"temporaryId":"history-tmp","channelId":common::cid(0x11f),"userId":"sender-1","type":"TEXT","message":"http-1","simpleMessage":"1","viewers":["viewer-1"],"props":{},"repliedMessage":reply,"createAt":10});
                            let data = if initial {
                                json!({"posts":[post]})
                            } else {
                                json!([post])
                            };
                            h.step(
                                common::port_reply_with_body(
                                    corr.raw(),
                                    common::enveloped(Bytes::from(
                                        serde_json::to_vec(
                                            &json!({"status":"SUCCESS","data":data}),
                                        )
                                        .unwrap(),
                                    )),
                                ),
                                now,
                            );
                        }
                        Effect::Persist { corr, ops } => {
                            assert!(
                                events.is_empty(),
                                "terminal cannot precede durable readback"
                            );
                            readback |= ops
                                .iter()
                                .any(|op| matches!(op, helix_core::effect::StorageOp::Scan(_)));
                            let outcome = execute_storage_ops(&db, ops).await;
                            assert!(matches!(outcome, PortOutcome::Ok(_)), "{outcome:?}");
                            // 本轮独占库中改变已提交正文，证明终态读取SQLite而非回显HTTP。
                            if ops.iter().any(|op| {
                                matches!(op, helix_core::effect::StorageOp::BatchUpsert(_))
                            }) {
                                db.execute_raw("UPDATE message SET message='1' WHERE temporary_id='history-tmp'").await.unwrap();
                            }
                            h.module
                                .handle(
                                    &Tick::PortReply {
                                        corr: *corr,
                                        outcome,
                                    },
                                    now,
                                    &mut h.sink,
                                )
                                .unwrap();
                        }
                        Effect::Emit { event } => {
                            assert!(readback);
                            events.push(serde_json::from_slice(&event.0).unwrap());
                        }
                        _ => panic!("unexpected navigation effect {fx:?}"),
                    }
                }
            }
            assert_eq!(http_count, 1);
            assert!(h.sink.is_empty());
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["event"], "im:read:result");
            let post = &events[0]["data"]["body"]["posts"][0];
            assert_eq!(post["id"], id);
            assert_eq!(post["message"], "1");
            assert_eq!(post["props"], json!({}));
            assert_eq!(post["topic"], json!({}));
            assert_eq!(post["replyMessages"], json!({}));
            let row = db
                .get(GetSpec {
                    table: "message",
                    key_col: "temporary_id",
                    key_val: SqlValue::Text("history-tmp".into()),
                })
                .await
                .unwrap()
                .unwrap();
            assert!(row.iter().any(|(key, _)| key == "replied_message"));
            if reply.is_object() {
                assert_eq!(post["repliedMessage"]["repliedUserId"], "sender-2");
                assert_eq!(post["repliedMessage"]["message"], "引用正文");
            } else {
                assert!(
                    post["repliedMessage"].is_null(),
                    "{initial}: no reply must stay null, got {}",
                    post["repliedMessage"]
                );
            }
            if !initial && reply.is_null() {
                if let Ok(dir) = std::env::var("HISTORY_REPLY_EVIDENCE_DIR") {
                    let p = std::path::Path::new(&dir);
                    std::fs::create_dir_all(p).unwrap();
                    std::fs::write(
                        p.join("exact-no-reply.json"),
                        serde_json::to_vec_pretty(&events[0]).unwrap(),
                    )
                    .unwrap();
                }
            }
        }
    }
}
