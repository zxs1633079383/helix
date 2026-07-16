//! UC15/UC16 原生命令 render-ready 投影：HTTP 权威回包必须先过本地持久屏障。

mod common;
use common::*;

use helix_core::effect::StorageOp;
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::{json, Value};

fn http_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("命令必须发 HTTP")
}

fn persist_corr_and_tables(h: &Harness) -> (u64, Vec<&'static str>) {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((
                corr.raw(),
                ops.iter()
                    .map(|op| match op {
                        StorageOp::BatchUpsert(spec) => spec.table,
                        StorageOp::BatchUpdate(spec) => spec.table,
                        other => panic!("unexpected persistence op: {other:?}"),
                    })
                    .collect(),
            )),
            _ => None,
        })
        .expect("权威回包必须形成带回执的持久屏障")
}

fn persisted_channel_value<'a>(
    h: &'a Harness,
    col: &str,
) -> Option<&'a helix_core::effect::SqlValue> {
    h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "channel" => {
                spec.rows.first().and_then(|row| {
                    row.iter()
                        .find(|(key, _)| key == col)
                        .map(|(_, value)| value)
                })
            }
            _ => None,
        }),
        _ => None,
    })
}

fn emit_data(h: &Harness, event_name: &str) -> Vec<Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|event| event["event"] == event_name)
        .map(|event| event["data"].clone())
        .collect()
}

fn inbound(value: Value) -> Tick {
    Tick::Inbound(InboundBytes(serde_json::to_vec(&value).unwrap().into()))
}

#[test]
fn make_topic_emits_replies_only_after_relation_persist_ack() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_make_topic",
            json!({
                "root_id": sid(15),
                "user_ids": ["u1"],
                "picture": "topic-picture"
            }),
        ),
        1_000,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "rootMessageId": sid(15),
                "channelId": cid(15),
                "revision": 41
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 1_010);

    assert!(emit_data(&h, "im:channel:replies").is_empty());
    let (persist_corr, tables) = persist_corr_and_tables(&h);
    assert_eq!(tables, vec!["channel", "message"]);

    h.step(
        port_reply_with_body(persist_corr, bytes::Bytes::new()),
        1_020,
    );
    let replies = emit_data(&h, "im:channel:replies");
    assert_eq!(replies.len(), 1);
    assert_eq!(replies[0]["rootMessageId"], sid(15));
    assert_eq!(replies[0]["channelId"], cid(15));
    assert_eq!(replies[0]["reqId"], format!("make-topic:{}", sid(15)));
    assert_eq!(replies[0]["revision"], 41);
    assert_eq!(replies[0]["projectionMode"], "snapshot");
}

#[test]
fn make_topic_rejects_failed_or_mismatched_authority_without_persist_or_projection() {
    for body in [
        json!({"status":"FAILED","data":{"code":"topic_failed"}}),
        json!({"status":"SUCCESS","data":{"rootMessageId":sid(99),"channelId":cid(15),"revision":1}}),
    ] {
        let mut h = Harness::new(&[]);
        h.step(
            command_json(
                "im_make_topic",
                json!({"root_id":sid(15),"user_ids":["u1"],"picture":"p"}),
            ),
            2_000,
        );
        let corr = http_corr(&h);
        let raw = bytes::Bytes::from(serde_json::to_vec(&body).unwrap());
        h.step(port_reply_with_body(corr, enveloped(raw)), 2_010);
        assert!(!h
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })));
        assert!(emit_data(&h, "im:channel:replies").is_empty());
    }
}

#[test]
fn settings_reply_emits_server_values_only_after_channel_persist_ack() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_channel_change_info",
            json!({
                "channel_id": cid(16),
                "purpose": "客户端请求值",
                "header": "客户端请求规则",
                "expected_version": 8
            }),
        ),
        3_000,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status":"SUCCESS",
            "data": {
                "channelId": cid(16),
                "purpose": "服务端权威简介",
                "header": "服务端权威规则",
                "settingVersion": 9
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 3_010);
    assert!(emit_data(&h, "im:channel:settings-updated").is_empty());
    assert!(matches!(
        persisted_channel_value(&h, "header"),
        Some(helix_core::effect::SqlValue::Text(value)) if value == "服务端权威规则"
    ));
    let (persist_corr, tables) = persist_corr_and_tables(&h);
    assert_eq!(tables, vec!["channel"]);

    h.step(
        port_reply_with_body(persist_corr, bytes::Bytes::new()),
        3_020,
    );
    let updates = emit_data(&h, "im:channel:settings-updated");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["channelId"], cid(16));
    assert_eq!(updates[0]["settings"]["purpose"], "服务端权威简介");
    assert_eq!(updates[0]["settings"]["header"], "服务端权威规则");
    assert_eq!(updates[0]["settingVersion"], 9);
}

#[test]
fn display_name_command_uses_same_authoritative_settings_persist_barrier() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_channel_change_display_name",
            json!({"channel_id":cid(16),"display_name":"新群名"}),
        ),
        3_100,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status":"SUCCESS",
            "data": {
                "channelId":cid(16),
                "purpose":"服务端简介",
                "header":"服务端规则",
                "settingVersion":12
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 3_110);
    assert!(emit_data(&h, "im:channel:settings-updated").is_empty());
    assert!(matches!(
        persisted_channel_value(&h, "header"),
        Some(helix_core::effect::SqlValue::Text(value)) if value == "服务端规则"
    ));
    let (persist_corr, _) = persist_corr_and_tables(&h);
    h.step(port_reply_ok(persist_corr), 3_120);
    assert_eq!(
        emit_data(&h, "im:channel:settings-updated")[0]["settingVersion"],
        12
    );
}

#[test]
fn notice_uses_same_authoritative_settings_reply_contract() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_channel_change_notice",
            json!({"channel_id":cid(16),"notice":{"content":"公告"}}),
        ),
        4_000,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status":"SUCCESS",
            "data": {
                "channelId": cid(16),
                "purpose": "简介",
                "header": "规则",
                "settingVersion": 10
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 4_010);
    let (persist_corr, _) = persist_corr_and_tables(&h);
    h.step(
        port_reply_with_body(persist_corr, bytes::Bytes::new()),
        4_020,
    );
    assert_eq!(
        emit_data(&h, "im:channel:settings-updated")[0]["settingVersion"],
        10
    );
}

#[test]
fn stale_expected_version_failure_never_persists_or_emits_settings_projection() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_channel_change_info",
            json!({"channel_id":cid(16),"purpose":"stale","expected_version":7}),
        ),
        5_000,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status":"FAILED",
            "data": {
                "code":"CHANNEL_SETTINGS_VERSION_CONFLICT",
                "expectedVersion":7,
                "currentVersion":9
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 5_010);
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Persist { .. })));
    assert!(emit_data(&h, "im:channel:settings-updated").is_empty());
}

#[test]
fn update_channel_notice_ws_requires_server_version_and_emits_authoritative_settings() {
    let mut h = Harness::new(&[]);
    h.step(
        inbound(json!({
            "action":"update_channel_notice",
            "data": {
                "channelId":cid(16),
                "purpose":"服务端简介",
                "header":"服务端规则",
                "settingVersion":11
            }
        })),
        6_000,
    );
    let updates = emit_data(&h, "im:channel:settings-updated");
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["settings"]["purpose"], "服务端简介");
    assert_eq!(updates[0]["settings"]["header"], "服务端规则");
    assert_eq!(updates[0]["settingVersion"], 11);

    h.step(
        inbound(json!({
            "action":"update_channel_notice",
            "data":{"channelId":cid(16),"purpose":"缺版本","header":"缺版本"}
        })),
        6_010,
    );
    assert!(emit_data(&h, "im:channel:settings-updated").is_empty());
}
