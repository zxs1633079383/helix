// 回归锚: Spec 047/T036-T037，G-06 quickReply durable 列与双视角绝对态。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::{command_json, port_reply_ok, port_reply_with_body, Harness};

const CHANNEL_ID: &str = "chfixx00000000000000000006";
const POST_ID: &str = "srvrea00000000000000000006";
const ACTOR_ID: &str = "g06-actor";
const RECEIVER_ID: &str = "g06-receiver";
const EMOJI: &str = ":thumbsup:";
const CREATE_AT: i64 = 1_700_000_600_000;
const UPDATE_AT: i64 = CREATE_AT + 60;

type TestResult<T> = Result<T, String>;

/// 保存一个 persona 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-06 sender/receiver 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
    durable_row: Row,
}

/// 构造 Go `post_update` 广播的 quickReply 绝对态。
fn reaction_authority() -> Value {
    reaction_authority_at(1)
}

/// 以指定 eventSeq 构造可用于乱序门控的 quickReply authority。
fn reaction_authority_at(event_seq: u64) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": CHANNEL_ID,
            "userId": ACTOR_ID,
            "createAt": CREATE_AT,
            "updateAt": UPDATE_AT,
            "message": "Gate G06 reaction",
            "type": "TEXT",
            "props": {"channel_event_seq": event_seq},
            "quickReply": [{
                "emoji": EMOJI,
                "userIds": [ACTOR_ID]
            }],
            "event_seq": event_seq
        }
    })
}

/// 构造填补 G-06 reaction 前置缺口的普通文本 authority。
fn preceding_post_authority() -> Value {
    json!({
        "action": "post",
        "data": {
            "channelId": CHANNEL_ID,
            "event_seq": 1,
            "post": {
                "id": "srvpre00000000000000000006",
                "temporaryId": "tmppre00000000000000000006",
                "channelId": CHANNEL_ID,
                "userId": ACTOR_ID,
                "type": "TEXT",
                "message": "G06 preceding post",
                "simpleMessage": "G06 preceding post",
                "props": {},
                "viewers": ["all"],
                "createAt": CREATE_AT - 1,
                "updateAt": CREATE_AT - 1
            }
        }
    })
}

/// 构造本地 message 表在原子提交后的真实读回行。
fn durable_row() -> Row {
    vec![
        ("id".to_string(), SqlValue::Text(POST_ID.to_string())),
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("event_seq".to_string(), SqlValue::Integer(1)),
        ("update_at".to_string(), SqlValue::Integer(UPDATE_AT)),
        (
            "quick_reply".to_string(),
            SqlValue::Text(json!([{"emoji": EMOJI, "userIds": [ACTOR_ID]}]).to_string()),
        ),
    ]
}

/// 从当前 Effect batch 解出全部 MessageV3 业务事件。
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

/// 断言 reaction 只写 message.quick_reply 与 cursor，不再寄生 props.quickReply。
fn assert_reaction_write_set(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-06 authority must persist atomically".to_string())?;
    let patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(&spec.patch),
        _ => None,
    });
    let patch = patch.ok_or_else(|| "G-06 authority must update message".to_string())?;
    let quick_reply = patch.iter().find_map(|(column, value)| {
        (column == "quick_reply")
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => serde_json::from_str::<Value>(value).ok(),
                _ => None,
            })
    });
    assert_eq!(
        quick_reply,
        Some(json!([{"emoji": EMOJI, "userIds": [ACTOR_ID]}]))
    );
    let props = patch.iter().find_map(|(column, value)| {
        (column == "props")
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => serde_json::from_str::<Value>(value).ok(),
                _ => None,
            })
    });
    assert!(
        props
            .as_ref()
            .is_none_or(|value| value.get("quickReply").is_none()),
        "quickReply must not be duplicated into props"
    );
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
    Ok(())
}

/// 对一个 persona 回放 command 可选项、WS authority、原子提交和 durable readback。
fn run_persona(viewer_id: &str, include_command: bool) -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[6], viewer_id, "g06-company", "", "", "");
    let mut ticks = Vec::new();
    if include_command {
        harness.step(
            command_json(
                "im_quick_reply",
                json!({"post_id": POST_ID, "emoji": EMOJI, "req_id": "g06-request"}),
            ),
            CREATE_AT as u64,
        );
        assert_eq!(
            harness.last_http_body(),
            Some(json!({"postId": POST_ID, "emoji": EMOJI}))
        );
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "missing quickReply HTTP".to_string())?;
        ticks.push(json!({
            "kind": "Command",
            "value": {
                "name": "im_quick_reply",
                "post_id": POST_ID,
                "emoji": EMOJI,
                "req_id": "g06-request"
            },
            "now_ms": CREATE_AT
        }));
        harness.step(
            port_reply_with_body(
                http_corr.raw(),
                common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
            ),
            CREATE_AT as u64 + 1,
        );
        ticks.push(json!({
            "kind": "PortReply",
            "value": {"correlation": http_corr.raw(), "outcome": {"variant": "Ok"}},
            "now_ms": CREATE_AT + 1
        }));
        assert!(
            emitted_events(harness.sink.as_slice())
                .iter()
                .all(|value| value["event"] == "im:read:result"),
            "HTTP result is not reaction authority"
        );
    }

    let authority = reaction_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        CREATE_AT as u64 + 2,
    );
    assert_reaction_write_set(harness.sink.as_slice())?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing reaction PersistAtomic".to_string())?;
    ticks.push(json!({
        "kind": "Inbound",
        "value": authority,
        "now_ms": CREATE_AT + 2
    }));

    harness.step(port_reply_ok(persist_corr.raw()), CREATE_AT as u64 + 3);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "PersistOk must schedule durable readback before terminal emit"
    );
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "reaction PersistOk must request message readback".to_string())?;
    ticks.push(json!({
        "kind": "PortReply",
        "value": {"correlation": persist_corr.raw(), "outcome": {"variant": "Ok"}},
        "now_ms": CREATE_AT + 3
    }));

    let row = durable_row();
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        CREATE_AT as u64 + 4,
    );
    ticks.push(json!({
        "kind": "PortReply",
        "value": {
            "correlation": readback_corr.raw(),
            "outcome": {
                "variant": "Ok",
                "rows": [{
                    "id": POST_ID,
                    "channel_id": CHANNEL_ID,
                    "event_seq": 1,
                    "update_at": UPDATE_AT,
                    "quick_reply": [{"emoji": EMOJI, "userIds": [ACTOR_ID]}]
                }]
            }
        },
        "now_ms": CREATE_AT + 4
    }));
    let events = emitted_events(harness.sink.as_slice());

    let duplicate = serde_json::to_vec(&reaction_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(duplicate))),
        CREATE_AT as u64 + 5,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same eventSeq replay must be idempotent"
    );
    ticks.push(json!({
        "kind": "Inbound",
        "value": reaction_authority(),
        "now_ms": CREATE_AT + 5
    }));
    Ok(PersonaTrace { events, ticks })
}

/// 回放 sender 与 receiver 两套独立本地状态。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_persona(ACTOR_ID, true)?,
        receiver: run_persona(RECEIVER_ID, false)?,
        durable_row: durable_row(),
    })
}

/// 断言一个 persona 只收到最小 quickReply 绝对态。
fn assert_terminal(events: &[Value]) {
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        json!({
            "event": "im:post:updated",
            "data": {
                "id": POST_ID,
                "channelId": CHANNEL_ID,
                "eventSeq": 1,
                "updateAt": UPDATE_AT,
                "quickReply": [{
                    "emoji": EMOJI,
                    "userIds": [ACTOR_ID]
                }]
            }
        })
    );
}

/// G-06 双视角只消费同一 durable quickReply 绝对态，重复 authority 不重复发布。
#[test]
fn g06_persists_and_emits_quick_reply_as_a_first_class_column() -> TestResult<()> {
    let trace = run_gate()?;
    assert_terminal(&trace.sender.events);
    assert_terminal(&trace.receiver.events);
    let encoded = serde_json::to_string(&trace.sender.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "actionCapabilities",
        "InteractionFrame",
        "Surface",
        "reactions",
        "reactionCount",
        "props",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// 乱序 reaction 必须等待前置 post 提交，再进入 G-06 原子写而非旧投影。
#[test]
fn g06_gap_buffer_resumes_without_legacy_projection() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[6], ACTOR_ID, "g06-company", "", "", "");
    let reaction =
        serde_json::to_vec(&reaction_authority_at(2)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(reaction))),
        CREATE_AT as u64,
    );
    assert!(
        correlation(harness.sink.as_slice(), "atomic").is_none(),
        "gap reaction must remain buffered"
    );
    assert!(
        harness
            .sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::PersistFire { .. } | Effect::Emit { .. })),
        "gap reaction must not bypass PersistOk through PersistFire or early Emit: {:?}",
        harness.sink.as_slice()
    );

    let preceding =
        serde_json::to_vec(&preceding_post_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(preceding))),
        CREATE_AT as u64 + 1,
    );
    let post_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "preceding post must start atomic persistence".to_string())?;
    harness.step(port_reply_ok(post_corr.raw()), CREATE_AT as u64 + 2);
    assert_reaction_write_set(harness.sink.as_slice())?;
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "resumed reaction cannot publish through the legacy projection"
    );
    Ok(())
}

/// 同一 durable message row 在无 WS/HTTP 输入时也能重建相同 reaction payload。
#[test]
fn g06_cold_start_row_rebuilds_the_same_terminal_event() -> TestResult<()> {
    let event = helix_im::event::post::reaction_update_from_row(&durable_row())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "durable reaction row must rebuild an event".to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("reaction factory must return an Emit effect".to_string());
    };
    let encoded =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_terminal(std::slice::from_ref(&encoded));
    Ok(())
}

/// Rust producer 在生成模式下写出 G-06 sender/receiver canonical artifact。
#[test]
fn g06_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-06",
        "message_v3_g06",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "channel_id", "quick_reply", "update_at"]},
            {"table": "channel_event", "columns": ["channel_id", "event_seq", "event_type", "msg_id"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "quick_reply", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "quick_reply", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
    );
    assert_eq!(trace.durable_row.len(), 5);
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// 取消最后一个回应时完整 post_update 的空数组必须清除 durable quick_reply。
#[test]
fn g06_empty_reaction_snapshot_clears_the_durable_column() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[6], ACTOR_ID, "g06-company", "", "", "");
    let mut authority = reaction_authority();
    authority["data"]["quickReply"] = json!([]);
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).unwrap(),
        ))),
        CREATE_AT as u64,
    );
    let patch = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            let Effect::PersistAtomic { ops, .. } = effect else {
                return None;
            };
            ops.iter().find_map(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(&spec.patch),
                _ => None,
            })
        })
        .ok_or("missing message patch")?;
    assert!(
        patch.iter().any(|(column, value)| column == "quick_reply"
            && matches!(value, SqlValue::Text(raw) if raw == "[]")),
        "empty authority must clear reaction column"
    );
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "no terminal before PersistOk"
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic").ok_or("missing persist")?;
    harness.step(port_reply_ok(persist_corr.raw()), CREATE_AT as u64 + 1);
    let events = emitted_events(harness.sink.as_slice());
    assert!(
        events
            .iter()
            .any(|event| event["event"] == "im:post:updated"
                && event["data"]["quickReply"] == json!([])),
        "persisted empty snapshot must reach UI"
    );
    Ok(())
}

/// 快捷回应的 HTTP 失败必须回传 req_id，避免客户端把 accepted 误当成功并悬挂。
#[test]
fn g06_http_business_failure_reaches_the_requester() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[6], ACTOR_ID, "g06-company", "", "", "");
    harness.step(
        command_json(
            "im_quick_reply",
            json!({"post_id": POST_ID, "emoji": EMOJI, "req_id": "g06-failure"}),
        ),
        CREATE_AT as u64,
    );
    let corr = correlation(harness.sink.as_slice(), "http").ok_or("missing HTTP")?;
    harness.step(
        port_reply_with_body(
            corr.raw(),
            common::enveloped(Bytes::from_static(
                br#"{"status":"FAIL","message":"denied"}"#,
            )),
        ),
        CREATE_AT as u64 + 1,
    );
    let events = emitted_events(harness.sink.as_slice());
    assert!(
        events.iter().any(|value| value["event"] == "im:read:result"
            && value["data"]["req_id"] == "g06-failure"
            && value["data"].get("error").is_some()),
        "business failure must reach requester"
    );
    assert!(!events
        .iter()
        .any(|value| value["event"] == "im:post:updated"));
    Ok(())
}
