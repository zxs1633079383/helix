// 回归锚: Spec 047/T042-T043，G-14a 模板确认 props durable 绝对态。

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

use common::{command_json, port_reply_err, port_reply_ok, port_reply_with_body, Harness};

const CHANNEL_ID: &str = "chfixx0000000000000000014a";
const POST_ID: &str = "srvtem0000000000000000014a";
const AUTHOR_ID: &str = "g14a-author";
const CONFIRMER_ID: &str = "g14a-confirmer";
const UPDATE_AT: i64 = 1_700_001_410_020;

type TestResult<T> = Result<T, String>;

/// 保存一个 persona 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-14a confirmer/author 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造远端 post.props 中的模板确认绝对集合。
fn template_props() -> Value {
    json!({"template": {"userIds": [CONFIRMER_ID]}})
}

/// 构造 Go `post_update` 广播的模板确认 authority。
fn template_authority(event_seq: u64) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": CHANNEL_ID,
            "userId": AUTHOR_ID,
            "createAt": UPDATE_AT - 20,
            "updateAt": UPDATE_AT,
            "message": "Gate G14a template",
            "type": "TEMPLATE",
            "props": template_props(),
            "event_seq": event_seq
        }
    })
}

/// 构造模板 props 提交后的本地 message 真实读回行。
fn durable_row(event_seq: u64) -> Row {
    vec![
        ("id".to_string(), SqlValue::Text(POST_ID.to_string())),
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("event_seq".to_string(), SqlValue::Integer(event_seq as i64)),
        ("update_at".to_string(), SqlValue::Integer(UPDATE_AT)),
        (
            "props".to_string(),
            SqlValue::Text(template_props().to_string()),
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

/// 断言模板确认只写 message.props/version 与 cursor。
fn assert_template_write_set(effects: &[Effect], event_seq: u64) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-14a authority must persist atomically".to_string())?;
    let patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(&spec.patch),
        _ => None,
    });
    let patch = patch.ok_or_else(|| "G-14a must update message".to_string())?;
    let props = patch.iter().find_map(|(column, value)| {
        (column == "props")
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => serde_json::from_str::<Value>(value).ok(),
                _ => None,
            })
    });
    assert_eq!(props, Some(template_props()));
    assert!(patch.iter().any(|(column, value)| {
        column == "event_seq"
            && matches!(value, SqlValue::Integer(value) if *value == event_seq as i64)
    }));
    for forbidden in [
        "message",
        "quick_reply",
        "expedite_map",
        "read_bits",
        "send_status",
    ] {
        assert!(
            patch.iter().all(|(column, _)| column != forbidden),
            "template write must not touch {forbidden}"
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
    Ok(())
}

/// 回放一次模板确认 authority 到 durable row 读回。
fn apply_authority(
    harness: &mut Harness,
    event_seq: u64,
    now_ms: u64,
    ticks: &mut Vec<Value>,
) -> TestResult<Vec<Value>> {
    let authority = template_authority(event_seq);
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), now_ms);
    assert_template_write_set(harness.sink.as_slice(), event_seq)?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing template PersistAtomic".to_string())?;
    ticks.push(json!({
        "kind": "Inbound",
        "value": authority,
        "now_ms": now_ms
    }));

    harness.step(port_reply_ok(persist_corr.raw()), now_ms + 1);
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "template PersistOk must request durable readback".to_string())?;
    let row = durable_row(event_seq);
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
                    "update_at": UPDATE_AT,
                    "props": template_props()
                }]
            }
        },
        "now_ms": now_ms + 2
    }));
    Ok(emitted_events(harness.sink.as_slice()))
}

/// 回放单个 persona 的 command admission 与可信 WS authority。
fn run_persona(viewer_id: &str, include_command: bool) -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[0x14a], viewer_id, "g14a-company", "", "", "");
    let mut ticks = Vec::new();
    if include_command {
        let payload = json!({"post_id": POST_ID});
        harness.step(
            command_json("im_template_received", payload.clone()),
            UPDATE_AT as u64,
        );
        assert_eq!(harness.last_http_body(), Some(json!({"postId": POST_ID})));
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "missing templateReceived HTTP".to_string())?;
        harness.step(port_reply_ok(http_corr.raw()), UPDATE_AT as u64 + 1);
        assert!(
            emitted_events(harness.sink.as_slice()).is_empty(),
            "HTTP ack is admission only"
        );
        ticks.push(json!({
            "kind": "Command",
            "value": {"name": "im_template_received", "payload": payload},
            "now_ms": UPDATE_AT
        }));
    }
    let events = apply_authority(&mut harness, 1, UPDATE_AT as u64 + 2, &mut ticks)?;
    let duplicate =
        serde_json::to_vec(&template_authority(1)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(duplicate))),
        UPDATE_AT as u64 + 6,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(PersonaTrace { events, ticks })
}

/// 回放 confirmer 与 author 两套独立本地状态。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_persona(CONFIRMER_ID, true)?,
        receiver: run_persona(AUTHOR_ID, false)?,
    })
}

/// 构造本 Gate 唯一允许公开的真实 props 绝对态。
fn expected_event() -> Value {
    json!({
        "event": "im:post:updated",
        "data": {
            "id": POST_ID,
            "channelId": CHANNEL_ID,
            "eventSeq": 1,
            "updateAt": UPDATE_AT,
            "props": template_props()
        }
    })
}

/// G-14a 双视角只收到相同的真实 props，不发布派生 UI 别名。
#[test]
fn g14a_emits_actual_template_props_for_both_personas() -> TestResult<()> {
    let trace = run_gate()?;
    assert_eq!(trace.sender.events, vec![expected_event()]);
    assert_eq!(trace.receiver.events, vec![expected_event()]);
    let encoded = serde_json::to_string(&trace.sender.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "actionCapabilities",
        "InteractionFrame",
        "Surface",
        "templateReceived",
        "templateConfirmedCount",
        "readerIds",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// G-14a 持久化失败不发布终态。
#[test]
fn g14a_persist_failure_emits_no_terminal_event() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[0x14a], AUTHOR_ID, "g14a-company", "", "", "");
    let inbound = serde_json::to_vec(&template_authority(1)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        UPDATE_AT as u64,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing template PersistAtomic".to_string())?;
    harness.step(port_reply_err(persist_corr.raw()), UPDATE_AT as u64 + 1);
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    Ok(())
}

/// G-14a 冷启动可从 durable props row 重建相同终态。
#[test]
fn g14a_cold_start_row_rebuilds_the_same_event() -> TestResult<()> {
    let event = helix_im::event::post::template_update_from_row(&durable_row(1))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "durable template row must rebuild an event".to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("template factory must return Emit".to_string());
    };
    let actual =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(actual, expected_event());
    Ok(())
}

/// G-14a 乱序 authority 必须等待缺失 seq，再按顺序进入原子写。
#[test]
fn g14a_gap_buffer_resumes_in_authority_order() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[0x14a], AUTHOR_ID, "g14a-company", "", "", "");
    let future = serde_json::to_vec(&template_authority(2)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(future))),
        UPDATE_AT as u64,
    );
    assert!(correlation(harness.sink.as_slice(), "atomic").is_none());
    let active = serde_json::to_vec(&template_authority(1)).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(active))),
        UPDATE_AT as u64 + 1,
    );
    let first_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "active template authority must persist first".to_string())?;
    harness.step(port_reply_ok(first_corr.raw()), UPDATE_AT as u64 + 2);
    assert_template_write_set(harness.sink.as_slice(), 2)?;
    Ok(())
}

/// Rust producer 在生成模式下写出 G-14a 双视角 canonical artifact。
#[test]
fn g14a_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-14a",
        "message_v3_g14a",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "channel_id", "props", "update_at"]},
            {"table": "channel_event", "columns": ["channel_id", "event_seq", "event_type", "msg_id"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "props", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "channel_id", "props", "event_seq", "update_at"]},
            {"table": "channel_event_cursor", "columns": ["channel_id", "last_event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
