// 回归锚: Spec 047/T030-T031，G-04 在线撤回与 lastPost 双视角绝对态。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::{command_json, port_reply_ok, Harness};

const CHANNEL_ID: &str = "chfixx00000000000000000004";
const POST_ID: &str = "srvrvk00000000000000000004";
const TEMPORARY_ID: &str = "tmprvk00000000000000000004";
const ACTOR_ID: &str = "g04-actor";
const RECEIVER_ID: &str = "g04-receiver";
const MESSAGE: &str = "Gate G04 revoke";
const CREATE_AT: i64 = 1_700_000_300_004;
const UPDATE_AT: i64 = CREATE_AT + 10;

type TestResult<T> = Result<T, String>;

struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造 Go 提交后广播的 posts_update 撤回 authority。
fn posts_update_authority() -> Value {
    json!({
        "action": "posts_update",
        "data": {
            "event_seq": 1,
            "channelId": CHANNEL_ID,
            "actor_id": ACTOR_ID,
            "system_event": {
                "eventId": "g04-revoke-event",
                "eventSeq": 1,
                "actorId": ACTOR_ID,
                "targetMsgId": POST_ID,
                "systemEvent": "message-recalled",
                "occurredAt": UPDATE_AT
            },
            "posts": [{
                "id": POST_ID,
                "temporaryId": TEMPORARY_ID,
                "channelId": CHANNEL_ID,
                "userId": ACTOR_ID,
                "type": "TEXT",
                "message": MESSAGE,
                "simpleMessage": MESSAGE,
                "props": {
                    "channel_event_seq": 1,
                    "system_event": {
                        "eventId": "g04-revoke-event",
                        "actorId": ACTOR_ID,
                        "systemEvent": "message-recalled"
                    }
                },
                "viewers": ["all"],
                "createAt": CREATE_AT,
                "updateAt": UPDATE_AT,
                "revoke": true
            }]
        }
    })
}

/// 构造 Go 仅在被撤回消息为 lastPost 时广播的 update_channel authority。
fn update_channel_authority() -> Value {
    let last_post = json!({
        "id": POST_ID,
        "temporaryId": TEMPORARY_ID,
        "channelId": CHANNEL_ID,
        "userId": ACTOR_ID,
        "type": "TEXT",
        "message": MESSAGE,
        "simpleMessage": MESSAGE,
        "props": {
            "channel_event_seq": 1,
            "system_event": {
                "eventId": "g04-revoke-event",
                "actorId": ACTOR_ID,
                "systemEvent": "message-recalled"
            }
        },
        "viewers": ["all"],
        "createAt": CREATE_AT,
        "updateAt": UPDATE_AT,
        "revoke": true
    });
    json!({
        "action": "update_channel",
        "data": {
            "id": CHANNEL_ID,
            "lastPost": last_post.to_string()
        }
    })
}

/// 从当前 Effect batch 解出全部业务事件。
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

/// 按 Effect 类型读取当前唯一 correlation。
fn correlation(effects: &[Effect], kind: &str) -> Option<Correlation> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(*corr),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(*corr),
        ("persist", Effect::Persist { corr, .. }) => Some(*corr),
        _ => None,
    })
}

/// 判断撤回 authority 的持久写集是否覆盖业务列且不触碰本地已读态。
fn assert_revoke_write_set(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-04 authority must persist atomically".to_string())?;
    let patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(&spec.patch),
        _ => None,
    });
    let patch = patch.ok_or_else(|| "G-04 authority must update the message row".to_string())?;
    for column in [
        "type",
        "message",
        "props",
        "viewers",
        "simple_message",
        "event_seq",
        "update_at",
        "revoke",
    ] {
        assert!(
            patch.iter().any(|(name, _)| name == column),
            "missing revoke authority column {column}"
        );
    }
    assert!(!patch.iter().any(|(name, _)| name == "read_bits"));
    assert!(!patch.iter().any(|(name, _)| name == "send_status"));
    Ok(())
}

/// 构造撤回 lastPost 提交后当前 viewer 的复合键绝对态读回。
fn member_reply(viewer_id: &str, unread_count: i64) -> ReplyBytes {
    let last_post = json!({
        "id": POST_ID,
        "temporaryId": TEMPORARY_ID,
        "channelId": CHANNEL_ID,
        "userId": ACTOR_ID,
        "type": "TEXT",
        "message": MESSAGE,
        "createAt": CREATE_AT,
        "updateAt": UPDATE_AT,
        "revoke": true
    });
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(viewer_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 回放一个 viewer 的 posts_update 与 update_channel 两段 authority。
fn run_persona(
    viewer_id: &str,
    unread_count: i64,
    include_command: bool,
) -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[4], viewer_id, "g04-company", "", "", "");
    let mut ticks = Vec::new();
    if include_command {
        harness.step(
            command_json("im_revoke", json!({"post_id": POST_ID})),
            CREATE_AT as u64,
        );
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "missing posts/revoke HTTP".to_string())?;
        ticks.push(json!({"kind": "Command", "value": {"name": "im_revoke", "post_id": POST_ID}, "now_ms": CREATE_AT}));
        harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
        ticks.push(json!({"kind": "PortReply", "value": {"correlation": http_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 1}));
        assert!(emitted_events(harness.sink.as_slice()).is_empty());
    }

    let posts_authority = posts_update_authority();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&posts_authority).map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 2,
    );
    assert_revoke_write_set(harness.sink.as_slice())?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let posts_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing revoke atomic persist".to_string())?;
    ticks.push(json!({"kind": "Inbound", "value": posts_authority, "now_ms": CREATE_AT + 2}));
    harness.step(port_reply_ok(posts_corr.raw()), CREATE_AT as u64 + 3);
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": posts_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 3}));
    let mut events = emitted_events(harness.sink.as_slice());

    let channel_authority = update_channel_authority();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&channel_authority).map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 4,
    );
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let channel_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing revoke channel atomic persist".to_string())?;
    ticks.push(json!({"kind": "Inbound", "value": channel_authority, "now_ms": CREATE_AT + 4}));
    harness.step(port_reply_ok(channel_corr.raw()), CREATE_AT as u64 + 5);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing revoke channel member readback".to_string())?;
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": channel_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 5}));
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, unread_count)),
        },
        CREATE_AT as u64 + 6,
    );
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": readback_corr.raw(), "outcome": {"variant": "Ok", "rows": [{"channel_id": CHANNEL_ID, "user_id": viewer_id, "unread_count": unread_count, "last_post": {"id": POST_ID, "temporaryId": TEMPORARY_ID, "channelId": CHANNEL_ID, "userId": ACTOR_ID, "type": "TEXT", "message": MESSAGE, "createAt": CREATE_AT, "updateAt": UPDATE_AT, "revoke": true}}]}}, "now_ms": CREATE_AT + 6}));
    events.extend(emitted_events(harness.sink.as_slice()));
    Ok(PersonaTrace { events, ticks })
}

/// 回放 sender 与 receiver 独立本地库的 G-04 双视角。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_persona(ACTOR_ID, 0, true)?,
        receiver: run_persona(RECEIVER_ID, 1, false)?,
    })
}

/// G-04 只发布 revoke 与写后 channel 绝对态，撤回不改变双方未读数。
#[test]
fn g04_revoke_is_idempotent_and_updates_last_post_for_both_personas() -> TestResult<()> {
    let trace = run_gate()?;
    for (events, unread_count) in [
        (&trace.sender.events, 0_i64),
        (&trace.receiver.events, 1_i64),
    ] {
        assert_eq!(
            events
                .iter()
                .map(|event| event["event"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["im:post:revoke", "im:channel:update"]
        );
        assert_eq!(events[0]["data"]["id"], POST_ID);
        assert_eq!(events[0]["data"]["revoke"], true);
        assert_eq!(events[1]["data"]["lastPost"]["id"], POST_ID);
        assert_eq!(events[1]["data"]["lastPost"]["revoke"], true);
        assert_eq!(events[1]["data"]["unreadCount"], unread_count);
        for event in events {
            let encoded = event.to_string();
            for forbidden in [
                "actionCapabilities",
                "actorName",
                "displayText",
                "systemEvent",
                "temporary_id",
            ] {
                assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
            }
        }
    }
    assert_eq!(
        trace.sender.events[0]["data"],
        trace.receiver.events[0]["data"]
    );
    Ok(())
}

/// Rust producer 在生成模式下写出 G-04 sender/receiver canonical artifact。
#[test]
fn g04_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-04",
        "message_v3_g04",
        trace.sender.ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "viewers", "revoke", "create_at", "update_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "viewers", "event_seq", "revoke", "create_at", "update_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "props", "viewers", "event_seq", "revoke", "create_at", "update_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
