// 回归锚: Spec 047/T038-T039，G-07 expedite_map durable 绝对态与双视角动作链。

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

const CHANNEL_ID: &str = "chfixx00000000000000000007";
const POST_ID: &str = "srvurg00000000000000000007";
const ACTOR_ID: &str = "g07-actor";
const RECEIVER_ID: &str = "g07-receiver";
const CREATE_AT: i64 = 1_700_000_700_000;

type TestResult<T> = Result<T, String>;

/// 保存一个 persona 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-07 sender/receiver 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造服务端真实 expediteMap 绝对结构。
fn expedite_map(confirmed: bool) -> Value {
    json!({
        "sender": {"id": ACTOR_ID},
        "recipients": {
            (RECEIVER_ID): {
                "status": if confirmed { 1 } else { 0 },
                "confirmAt": if confirmed { Some(CREATE_AT + 20) } else { None }
            }
        }
    })
}

/// 构造 Go `post_update` 广播的加急或确认 authority。
fn urgent_authority(event_seq: u64, confirmed: bool) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": CHANNEL_ID,
            "userId": ACTOR_ID,
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT + (event_seq as i64 * 10),
            "message": "Gate G07 urgent",
            "type": "TEXT",
            "props": {"channel_event_seq": event_seq},
            "expediteMap": expedite_map(confirmed),
            "event_seq": event_seq
        }
    })
}

/// 构造本地 message 表在加急提交后的真实读回行。
fn durable_row(event_seq: u64, confirmed: bool) -> Row {
    vec![
        ("id".to_string(), SqlValue::Text(POST_ID.to_string())),
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("event_seq".to_string(), SqlValue::Integer(event_seq as i64)),
        (
            "update_at".to_string(),
            SqlValue::Integer(CREATE_AT + (event_seq as i64 * 10)),
        ),
        (
            "expedite_map".to_string(),
            SqlValue::Text(expedite_map(confirmed).to_string()),
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

/// 断言加急只写 message 权威列与 cursor，不寄生 props 或内容列。
fn assert_urgent_write_set(effects: &[Effect], event_seq: u64, confirmed: bool) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-07 authority must persist atomically".to_string())?;
    let patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(&spec.patch),
        _ => None,
    });
    let patch = patch.ok_or_else(|| "G-07 authority must update message".to_string())?;
    let expedite = patch.iter().find_map(|(column, value)| {
        (column == "expedite_map")
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => serde_json::from_str::<Value>(value).ok(),
                _ => None,
            })
    });
    assert_eq!(expedite, Some(expedite_map(confirmed)));
    assert!(patch.iter().any(|(column, value)| {
        column == "event_seq"
            && matches!(value, SqlValue::Integer(value) if *value == event_seq as i64)
    }));
    for forbidden in [
        "message",
        "props",
        "quick_reply",
        "read_bits",
        "send_status",
    ] {
        assert!(
            patch.iter().all(|(column, _)| column != forbidden),
            "urgent write must not touch {forbidden}"
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
    Ok(())
}

/// 回放并提交一次加急 authority，返回 durable readback 后的唯一事件。
fn apply_authority(
    harness: &mut Harness,
    event_seq: u64,
    confirmed: bool,
    now_ms: u64,
    ticks: &mut Vec<Value>,
) -> TestResult<Vec<Value>> {
    let authority = urgent_authority(event_seq, confirmed);
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), now_ms);
    assert_urgent_write_set(harness.sink.as_slice(), event_seq, confirmed)?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing urgent PersistAtomic".to_string())?;
    ticks.push(json!({
        "kind": "Inbound",
        "value": authority,
        "now_ms": now_ms
    }));

    harness.step(port_reply_ok(persist_corr.raw()), now_ms + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "PersistOk must schedule durable readback before terminal emit"
    );
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "urgent PersistOk must request message readback".to_string())?;
    let row = durable_row(event_seq, confirmed);
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        now_ms + 2,
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
                    "event_seq": event_seq,
                    "update_at": CREATE_AT + (event_seq as i64 * 10),
                    "expedite_map": expedite_map(confirmed)
                }]
            }
        },
        "now_ms": now_ms + 2
    }));
    Ok(emitted_events(harness.sink.as_slice()))
}

/// 回放单个 persona 的 action admission 与两次权威绝对态。
fn run_persona(viewer_id: &str, sender: bool) -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[7], viewer_id, "g07-company", "", "", "");
    let command = if sender {
        (
            "im_urgent_post",
            json!({
                "post_id": POST_ID,
                "target_ids": [RECEIVER_ID],
                "req_id": "g07-post-request"
            }),
            json!({"postId": POST_ID, "targetIds": [RECEIVER_ID]}),
        )
    } else {
        (
            "im_urgent_confirm",
            json!({"post_id": POST_ID, "req_id": "g07-confirm-request"}),
            json!({"postId": POST_ID}),
        )
    };
    harness.step(command_json(command.0, command.1.clone()), CREATE_AT as u64);
    assert_eq!(harness.last_http_body(), Some(command.2));
    let http_corr = correlation(harness.sink.as_slice(), "http")
        .ok_or_else(|| "missing urgent action HTTP".to_string())?;
    let mut ticks = vec![json!({
        "kind": "Command",
        "value": {"name": command.0, "payload": command.1},
        "now_ms": CREATE_AT
    })];
    harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "HTTP ack is admission only"
    );

    let mut events = apply_authority(&mut harness, 1, false, CREATE_AT as u64 + 2, &mut ticks)?;
    events.extend(apply_authority(
        &mut harness,
        2,
        true,
        CREATE_AT as u64 + 6,
        &mut ticks,
    )?);

    let duplicate =
        serde_json::to_vec(&urgent_authority(2, true)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(duplicate))),
        CREATE_AT as u64 + 10,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same eventSeq replay must be idempotent"
    );
    Ok(PersonaTrace { events, ticks })
}

/// 回放 sender 与 receiver 两套独立本地状态。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_persona(ACTOR_ID, true)?,
        receiver: run_persona(RECEIVER_ID, false)?,
    })
}

/// 构造某一版本应公开的最小 expediteMap 绝对态。
fn expected_event(event_seq: u64, confirmed: bool) -> Value {
    json!({
        "event": "im:post:updated",
        "data": {
            "id": POST_ID,
            "channelId": CHANNEL_ID,
            "eventSeq": event_seq,
            "updateAt": CREATE_AT + (event_seq as i64 * 10),
            "expediteMap": expedite_map(confirmed)
        }
    })
}

/// 构造同一次 authority 的当前 viewer 成员级加急信号。
///
/// 本 fixture 的 `expediteMap.sender` 只有 id 没有 name，因此 `urgentCurrentName` 走空串降级。
fn expected_channel_signal(viewer_id: &str, pending: bool) -> Value {
    json!({
        "event": "im:channel:update",
        "data": {
            "channelId": CHANNEL_ID,
            "userId": viewer_id,
            "projectionAuthority": "server-member-absolute",
            "memberScope": "current_user",
            "urgentPostList": if pending { json!([POST_ID]) } else { json!([]) },
            "urgentCurrentName": "",
            "hasUrgentPost": pending,
        }
    })
}

/// G-07 双视角按同一 absolute map 分别解读 pending 与 completed。
#[test]
fn g07_emits_absolute_expedite_map_for_sender_and_receiver() -> TestResult<()> {
    let trace = run_gate()?;
    // 每次 authority 的精确出站序列 = post 绝对投影 → 当前成员的会话级信号。
    let expected_sender = vec![
        expected_event(1, false),
        expected_channel_signal(ACTOR_ID, false),
        expected_event(2, true),
        expected_channel_signal(ACTOR_ID, false),
    ];
    let expected_receiver = vec![
        expected_event(1, false),
        expected_channel_signal(RECEIVER_ID, true),
        expected_event(2, true),
        expected_channel_signal(RECEIVER_ID, false),
    ];
    assert_eq!(trace.sender.events, expected_sender);
    assert_eq!(trace.receiver.events, expected_receiver);
    let encoded = serde_json::to_string(&trace.sender.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "actionCapabilities",
        "InteractionFrame",
        "Surface",
        "urgentRequesterId",
        "urgentTargetIds",
        "urgentConfirmedIds",
        "urgentState",
        "props",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// G-07 冷启动只凭 durable message row 重建同一 completed payload。
#[test]
fn g07_cold_start_row_rebuilds_the_same_completed_event() -> TestResult<()> {
    let event = helix_im::event::post::urgent_update_from_row(&durable_row(2, true))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "durable urgent row must rebuild an event".to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("urgent factory must return an Emit effect".to_string());
    };
    let encoded =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(encoded, expected_event(2, true));
    Ok(())
}

/// G-07 乱序确认必须等待 active authority 提交后再进入第二次原子写。
#[test]
fn g07_gap_buffer_resumes_in_authority_order() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[7], ACTOR_ID, "g07-company", "", "", "");
    let confirmed =
        serde_json::to_vec(&urgent_authority(2, true)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(confirmed))),
        CREATE_AT as u64,
    );
    assert!(correlation(harness.sink.as_slice(), "atomic").is_none());

    let active =
        serde_json::to_vec(&urgent_authority(1, false)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(active))),
        CREATE_AT as u64 + 1,
    );
    let first_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "active authority must persist first".to_string())?;
    harness.step(port_reply_ok(first_corr.raw()), CREATE_AT as u64 + 2);
    assert_urgent_write_set(harness.sink.as_slice(), 2, true)?;
    Ok(())
}

/// Rust producer 在生成模式下写出 G-07 sender/receiver canonical artifact。
#[test]
fn g07_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-07",
        "message_v3_g07",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "channel_id", "expedite_map", "update_at"]},
            {"table": "channel_event", "columns": ["channel_id", "event_seq", "event_type", "msg_id"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "expedite_map", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "expedite_map", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
