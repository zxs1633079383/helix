// 回归锚: Spec 047/T024-T025，G-01 文本消息双 persona 持久后绝对态。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::Harness;

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const POST_ID: &str = "srvfix00000000000000000001";
const TEMPORARY_ID: &str = "tmpfix00000000000000000001";
const ACTOR_ID: &str = "g01-actor";
const RECEIVER_ID: &str = "g01-receiver";
const MESSAGE: &str = "Gate G01 MessageV3";
const CREATE_AT: i64 = 1_700_000_000_002;

struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
    final_unread: i64,
}

type TestResult<T> = Result<T, String>;

/// 构造真 Go `post` action 的普通文本 authority。
fn authority() -> Value {
    json!({
        "action": "post",
        "data": {
            "channelId": CHANNEL_ID,
            "event_seq": 1,
            "post": {
                "id": POST_ID,
                "temporaryId": TEMPORARY_ID,
                "channelId": CHANNEL_ID,
                "userId": ACTOR_ID,
                "type": "TEXT",
                "message": MESSAGE,
                "simpleMessage": MESSAGE,
                "props": {},
                "viewers": ["all"],
                "createAt": CREATE_AT,
                "updateAt": CREATE_AT
            }
        }
    })
}

/// 从当前 Effect batch 读取唯一相关持久请求。
fn correlated_persist(effects: &[Effect], atomic: bool) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match (atomic, effect) {
        (false, Effect::Persist { corr, ops }) => Some((*corr, ops.as_slice())),
        (true, Effect::PersistAtomic { corr, ops }) => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 从当前 Effect batch 读取唯一 ACK HTTP，并解出其业务请求体。
fn correlated_client_ack(effects: &[Effect]) -> Option<(Correlation, Value)> {
    effects.iter().find_map(|effect| {
        let Effect::Http { corr, req } = effect else {
            return None;
        };
        if !req.url.ends_with("/post/clientAck") {
            return None;
        }
        let body = req.body.as_ref()?;
        serde_json::from_slice(body.as_ref())
            .ok()
            .map(|body| (*corr, body))
    })
}

/// 返回写集涉及的表，证明 message/channel/member/cursor 同一原子提交。
fn write_tables(ops: &[StorageOp]) -> BTreeSet<&'static str> {
    ops.iter()
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(spec) => Some(spec.table),
            StorageOp::BatchUpdate(spec) => Some(spec.table),
            StorageOp::MonotonicUpsert(spec) => Some(spec.table),
            StorageOp::GuardedBump(spec) => Some(spec.table),
            StorageOp::ScopedGuardedBump(spec) => Some(spec.table),
            _ => None,
        })
        .collect()
}

/// 编码当前 viewer 的 channel_member 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64, last_post: &Value) -> ReplyBytes {
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        ("projection_revision".to_string(), SqlValue::Integer(13)),
        (
            "effect_id".to_string(),
            SqlValue::Text("server-effect-13".to_string()),
        ),
        (
            "unread_post_id".to_string(),
            SqlValue::Text(if unread_count == 0 {
                String::new()
            } else {
                POST_ID.to_string()
            }),
        ),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
        ("last_post_at".to_string(), SqlValue::Integer(CREATE_AT)),
        (
            "last_root_post_at".to_string(),
            SqlValue::Integer(CREATE_AT),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 从当前 Effect batch 解出生产 MessageV3 envelopes。
fn emitted_events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

/// 以独立本地库回放 sender 或 receiver 的读、写、读回和终态 Emit。
fn run_persona(viewer_id: &str, initial_unread: i64) -> TestResult<PersonaTrace> {
    let inbound = authority();
    let mut harness = Harness::new_with_identity(&[1], viewer_id, "g01-company", "", "", "");
    let inbound_bytes = serde_json::to_vec(&inbound).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound_bytes))),
        CREATE_AT as u64,
    );
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    let (persist_corr, writes) = correlated_persist(harness.sink.as_slice(), true)
        .ok_or_else(|| "missing G-01 atomic persistence".to_string())?;
    // POST 只可在 legacy revision=0 引导摘要；已有 server revision 的行必须保留同版本事实。
    assert!(writes.iter().any(|op| matches!(op,
        StorageOp::ScopedGuardedBump(spec) if spec.table == "channel_member"
            && matches!(&spec.additional_guard, Some(("projection_revision", SqlValue::Integer(0))))
    )));
    let tables = write_tables(writes);
    for required in [
        "message",
        "channel",
        "channel_member",
        "channel_event_cursor",
    ] {
        assert!(tables.contains(required), "missing atomic table {required}");
    }
    if viewer_id == ACTOR_ID {
        assert!(writes.iter().any(|op| {
            matches!(
                op,
                StorageOp::BatchUpsert(spec)
                    if spec.table == "channel_member"
                        && spec.rows.iter().any(|row| row.iter().any(|(column, value)| {
                            column == "unread_count"
                                && matches!(value, SqlValue::Integer(count) if *count == 0)
                        }))
            )
        }));
    } else {
        assert!(writes.iter().any(|op| {
            matches!(
                op,
                StorageOp::ScopedGuardedBump(spec)
                    if spec.table == "channel_member"
                        && spec.bump_col == "unread_count"
                        && spec.bump_delta == 1
            )
        }));
    }

    let final_unread = if viewer_id == ACTOR_ID {
        0
    } else {
        initial_unread + 1
    };
    harness.step(
        Tick::PortReply {
            corr: persist_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        CREATE_AT as u64 + 1,
    );
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    let (readback_corr, readback_ops) = correlated_persist(harness.sink.as_slice(), false)
        .ok_or_else(|| "missing G-01 scoped readback".to_string())?;
    assert!(readback_ops
        .iter()
        .any(|op| matches!(op, StorageOp::ScopedGet(spec) if spec.table == "channel_member")));

    let last_post = json!({
        "id": POST_ID,
        "userId": ACTOR_ID,
        "message": MESSAGE,
        "createAt": CREATE_AT
    });
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, final_unread, &last_post)),
        },
        CREATE_AT as u64 + 2,
    );
    let (ack_corr, ack_body) = correlated_client_ack(harness.sink.as_slice())
        .ok_or_else(|| "missing durable client ACK HTTP".to_string())?;
    assert_eq!(ack_body["postId"], POST_ID);
    assert_eq!(ack_body["ackId"], format!("{POST_ID}:1"));
    assert_eq!(ack_body["platform"], "native");
    let events = emitted_events(harness.sink.as_slice());
    harness.step(
        Tick::PortReply {
            corr: ack_corr,
            outcome: PortOutcome::Ok(helix_core::tick::ReplyBytes(common::enveloped(
                Bytes::from(
                    serde_json::to_vec(&json!({
                        "status": "SUCCESS",
                        "data": {"recorded": true}
                    }))
                    .map_err(|error| error.to_string())?,
                ),
            ))),
        },
        CREATE_AT as u64 + 3,
    );
    let ack_events = emitted_events(harness.sink.as_slice());
    assert_eq!(ack_events.len(), 1);
    assert_eq!(ack_events[0]["event"], "im:post:client-ack-succeeded");
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({"kind": "Inbound", "value": inbound, "now_ms": CREATE_AT}),
            json!({"kind": "PortReply", "value": {"correlation": persist_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 1}),
            json!({"kind": "PortReply", "value": {"correlation": readback_corr.raw(), "outcome": {"variant": "Ok", "rows": [{"channel_id": CHANNEL_ID, "user_id": viewer_id, "unread_count": final_unread, "last_post": last_post}]}}, "now_ms": CREATE_AT + 2}),
        ],
        final_unread,
    })
}

/// G-01 sender 与 receiver 收到同一 post，但 channel 未读绝对态分别为 0 和 N+1。
#[test]
fn g01_emits_committed_post_and_persona_channel_absolute_state() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, 7)?;
    let receiver = run_persona(RECEIVER_ID, 4)?;
    for trace in [&sender, &receiver] {
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(trace.events[0]["data"]["temporaryId"], TEMPORARY_ID);
        assert_eq!(trace.events[0]["data"]["sendStatus"], "sent");
        assert!(trace.events[0]["data"].get("props").is_none());
        assert_eq!(trace.events[1]["data"]["channelId"], CHANNEL_ID);
        assert_eq!(trace.events[1]["data"]["projectionRevision"], 13);
        assert!(trace.events[1]["data"]["userId"].is_string());
        assert_eq!(trace.events[1]["data"]["lastPostAt"], CREATE_AT);
        assert_eq!(trace.events[1]["data"]["lastRootPostAt"], CREATE_AT);
        assert_eq!(trace.events[1]["data"]["lastPost"]["id"], POST_ID);
        assert!(trace.events[1]["data"]["lastPost"].get("props").is_none());
        assert_eq!(trace.events[1]["data"]["unreadCount"], trace.final_unread);
        let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
        assert!(!encoded.contains("temporary_id"));
        assert!(!encoded.contains("actionCapabilities"));
        assert!(!encoded.contains("InteractionFrame"));
        assert!(!encoded.contains("Surface"));
    }
    assert_eq!(sender.final_unread, 0);
    assert_eq!(receiver.final_unread, 5);
    Ok(())
}

/// Rust producer 在生成模式下写出 G-01 双视角 canonical artifact。
#[test]
fn g01_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, 7)?;
    let receiver = run_persona(RECEIVER_ID, 4)?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-01",
        "message_v3_g01",
        sender.ticks,
        sender.events,
        receiver.events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "create_at"]},
            {"table": "channel", "columns": ["id", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post", "last_post_at", "last_root_post_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]},
            {"table": "channel", "columns": ["id", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]},
            {"table": "channel", "columns": ["id", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post", "last_post_at", "last_root_post_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
