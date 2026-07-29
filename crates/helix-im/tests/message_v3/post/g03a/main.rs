// 回归锚: Spec 047/T028-T029，G-03a 失败消息重试与双 persona 可见性。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::{command_json, port_reply_err, port_reply_ok, send_message_cmd_json, Harness};

const CHANNEL_ID: &str = "chfixx00000000000000000003";
const POST_ID: &str = "srvfix00000000000000000003";
const TEMPORARY_ID: &str = "tmpretry000000000000000003";
const ACTOR_ID: &str = "g03a-actor";
const RECEIVER_ID: &str = "g03a-receiver";
const MESSAGE: &str = "Gate G03a retry";
const CREATE_AT: i64 = 1_700_000_200_003;

type TestResult<T> = Result<T, String>;

struct GateTrace {
    sender_events: Vec<Value>,
    receiver_events: Vec<Value>,
    ticks: Vec<Value>,
}

struct AuthoritySettlement {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 构造 retry 成功后由 Go 广播的同 identity authority。
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

/// 从当前 Effect batch 解出所有生产 MessageV3 事件。
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

/// 按 Effect 类型读取相关端口 correlation。
fn correlation(effects: &[Effect], kind: &str) -> Option<Correlation> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("persist", Effect::Persist { corr, .. }) => Some(*corr),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(*corr),
        ("http", Effect::Http { corr, .. }) => Some(*corr),
        _ => None,
    })
}

/// 编码 authority 提交后当前 viewer 的 channel_member 绝对态。
fn member_row(viewer_id: &str, unread_count: i64) -> Value {
    json!({
        "channel_id": CHANNEL_ID,
        "user_id": viewer_id,
        "unread_count": unread_count,
        "last_post": {
            "id": POST_ID,
            "userId": ACTOR_ID,
            "message": MESSAGE,
            "createAt": CREATE_AT
        }
    })
}

/// 把 reviewer 可读的绝对态行编码为生产 PortReply bytes。
fn member_reply(viewer_id: &str, unread_count: i64) -> ReplyBytes {
    let row_value = member_row(viewer_id, unread_count);
    let last_post = row_value["last_post"].clone();
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

/// 把成功 authority 走完原子提交与 viewer 读回，并保留全部真实 Tick。
fn settle_authority(
    harness: &mut Harness,
    viewer_id: &str,
    unread_count: i64,
    now_ms: u64,
) -> TestResult<AuthoritySettlement> {
    let authority_value = authority();
    let bytes = serde_json::to_vec(&authority_value).map_err(|error| error.to_string())?;
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(bytes))), now_ms);
    let atomic_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing authority atomic persist".to_string())?;
    assert!(!emitted_events(harness.sink.as_slice())
        .iter()
        .any(|event| event["event"] == "im:post:received"));

    harness.step(port_reply_ok(atomic_corr.raw()), now_ms + 1);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing authority member readback".to_string())?;
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, unread_count)),
        },
        now_ms + 2,
    );
    Ok(AuthoritySettlement {
        events: emitted_events(harness.sink.as_slice()),
        ticks: vec![
            json!({"kind": "Inbound", "value": authority_value, "now_ms": now_ms}),
            json!({"kind": "PortReply", "value": {"correlation": atomic_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": now_ms + 1}),
            json!({"kind": "PortReply", "value": {"correlation": readback_corr.raw(), "outcome": {"variant": "Ok", "rows": [member_row(viewer_id, unread_count)]}}, "now_ms": now_ms + 2}),
        ],
    })
}

/// 运行首次失败、同 identity retry 与成功 authority 的 sender/receiver 回放。
fn run_gate() -> TestResult<GateTrace> {
    let mut sender = Harness::new_with_identity(&[3], ACTOR_ID, "g03a-company", "", "", "");
    sender.step(
        send_message_cmd_json(json!({
            "channel_id": CHANNEL_ID,
            "temporary_id": TEMPORARY_ID,
            "text": MESSAGE
        })),
        CREATE_AT as u64 - 3,
    );
    let initial_events = emitted_events(sender.sink.as_slice());
    assert_eq!(initial_events.len(), 1);
    assert_eq!(initial_events[0]["event"], "im:post:sending");
    assert_eq!(initial_events[0]["data"]["temporaryId"], TEMPORARY_ID);
    let mut sender_events = Vec::new();
    let initial_persist = correlation(sender.sink.as_slice(), "persist")
        .ok_or_else(|| "missing initial optimistic persist".to_string())?;

    sender.step(port_reply_ok(initial_persist.raw()), CREATE_AT as u64 - 2);
    let initial_http = correlation(sender.sink.as_slice(), "http")
        .ok_or_else(|| "missing initial posts/create".to_string())?;
    sender.step(port_reply_err(initial_http.raw()), CREATE_AT as u64 - 1);
    sender_events.extend(emitted_events(sender.sink.as_slice()));

    sender.step(
        command_json(
            "im_retry_send",
            json!({"message_id": TEMPORARY_ID, "req_id": "g03a-review-retry"}),
        ),
        CREATE_AT as u64,
    );
    let retry_persist = correlation(sender.sink.as_slice(), "persist")
        .ok_or_else(|| "missing retry sending persist".to_string())?;
    assert!(!sender
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
    sender_events.extend(emitted_events(sender.sink.as_slice()));

    sender.step(port_reply_ok(retry_persist.raw()), CREATE_AT as u64 + 1);
    let retry_body = sender
        .last_http_body()
        .ok_or_else(|| "missing retry posts/create body".to_string())?;
    assert_eq!(retry_body["temporaryId"], TEMPORARY_ID);
    let sender_settlement = settle_authority(&mut sender, ACTOR_ID, 0, CREATE_AT as u64 + 2)?;
    sender_events.extend(sender_settlement.events);

    let mut receiver = Harness::new_with_identity(&[3], RECEIVER_ID, "g03a-company", "", "", "");
    let receiver_events =
        settle_authority(&mut receiver, RECEIVER_ID, 1, CREATE_AT as u64 + 2)?.events;
    let mut ticks = vec![
        json!({"kind": "Command", "value": {"name": "im_send_message", "channel_id": CHANNEL_ID, "temporary_id": TEMPORARY_ID, "message": MESSAGE}, "now_ms": CREATE_AT - 3}),
        json!({"kind": "PortReply", "value": {"correlation": initial_persist.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT - 2}),
        json!({"kind": "PortReply", "value": {"correlation": initial_http.raw(), "outcome": {"variant": "Err", "code": 500}}, "now_ms": CREATE_AT - 1}),
        json!({"kind": "Command", "value": {"name": "im_retry_send", "message_id": TEMPORARY_ID, "req_id": "g03a-review-retry"}, "now_ms": CREATE_AT}),
        json!({"kind": "PortReply", "value": {"correlation": retry_persist.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 1}),
    ];
    ticks.extend(sender_settlement.ticks);
    Ok(GateTrace {
        sender_events,
        receiver_events,
        ticks,
    })
}

/// G-03a 仅 sender 看见 failed/retry，成功后双视角才共享 authority。
#[test]
fn g03a_reuses_identity_and_delays_receiver_until_success() -> TestResult<()> {
    let trace = run_gate()?;
    let sender_names = trace
        .sender_events
        .iter()
        .map(|event| event["event"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        sender_names,
        [
            "im:post:send-failed",
            "im:post:sending",
            "im:post:received",
            "im:channel:update"
        ]
    );
    for event in &trace.sender_events[..2] {
        assert_eq!(event["data"]["temporaryId"], TEMPORARY_ID);
        assert!(event["data"].get("temporary_id").is_none());
        assert!(event["data"].get("actionCapabilities").is_none());
    }
    assert_eq!(
        trace.sender_events[0]["data"],
        json!({
            "channelId": CHANNEL_ID,
            "temporaryId": TEMPORARY_ID,
            "sendStatus": "failed"
        })
    );
    assert_eq!(
        trace.sender_events[1]["data"],
        json!({
            "channelId": CHANNEL_ID,
            "temporaryId": TEMPORARY_ID,
            "userId": ACTOR_ID,
            "message": MESSAGE,
            "type": "TEXT",
            "createAt": CREATE_AT - 3,
            "sendStatus": "sending"
        })
    );
    assert_eq!(
        trace
            .receiver_events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["im:post:received", "im:channel:update"]
    );
    assert_eq!(trace.receiver_events[1]["data"]["unreadCount"], 1);
    Ok(())
}

/// Rust producer 在生成模式下写出 G-03a sender/receiver canonical artifact。
#[test]
fn g03a_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-03a",
        "message_v3_g03a",
        trace.ticks,
        trace.sender_events,
        trace.receiver_events,
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "unread_count", "last_post"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
