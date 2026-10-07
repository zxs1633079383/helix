// 回归锚: Spec 047/T044-T045，G-08 creator-only schedule durable 绝对态。

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

const CHANNEL_ID: &str = "chfixx00000000000000000008";
const SCHEDULE_ID: &str = "schedule-g08-1";
const CREATOR_ID: &str = "g08-creator";
const OBSERVER_ID: &str = "g08-observer";
const MESSAGE: &str = "G08 scheduled message";
const PREVIEW: &str = "G08 scheduled preview";
const SEND_AT: i64 = 1_700_000_860_000;
const REVISION: u64 = 8;

type TestResult<T> = Result<T, String>;

/// 保存 creator 或 observer 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-08 双 persona 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造 Go `post_schedule_created` 权威帧。
fn created_authority() -> Value {
    json!({
        "action": "post_schedule_created",
        "eventSeq": REVISION,
        "data": {
            "channelId": CHANNEL_ID,
            "scheduleId": SCHEDULE_ID,
            "scheduleOwnerUserId": CREATOR_ID,
            "message": MESSAGE,
            "messagePreview": PREVIEW,
            "sendAt": SEND_AT,
            "status": "scheduled",
            "revision": REVISION
        }
    })
}

/// 构造 creator 本地 channel_schedule 真实读回行。
fn durable_row() -> Row {
    vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        (
            "schedule_id".to_string(),
            SqlValue::Text(SCHEDULE_ID.to_string()),
        ),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(CREATOR_ID.to_string()),
        ),
        ("message".to_string(), SqlValue::Text(MESSAGE.to_string())),
        (
            "message_preview".to_string(),
            SqlValue::Text(PREVIEW.to_string()),
        ),
        ("send_at".to_string(), SqlValue::Integer(SEND_AT)),
        (
            "status".to_string(),
            SqlValue::Text("scheduled".to_string()),
        ),
        ("revision".to_string(), SqlValue::Integer(REVISION as i64)),
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

/// 断言 G-08 同步写 channel_schedule 与 channel creator read model。
fn assert_schedule_write_set(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-08 authority must persist atomically".to_string())?;
    assert_eq!(ops.len(), 2);
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::BatchUpsert(spec) if spec.table == "channel_schedule"
    )));
    let channel_patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(&spec.patch),
        _ => None,
    });
    let channel_patch =
        channel_patch.ok_or_else(|| "G-08 must update channel read model".to_string())?;
    assert!(channel_patch.iter().any(|(column, value)| {
        column == "has_schedule_post" && matches!(value, SqlValue::Integer(1))
    }));
    assert!(channel_patch.iter().any(|(column, value)| {
        column == "schedule_revision"
            && matches!(value, SqlValue::Integer(value) if *value == REVISION as i64)
    }));
    Ok(())
}

/// 回放 creator command、WS authority、PersistOk 与 durable readback。
fn run_creator() -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[8], CREATOR_ID, "g08-company", "", "", "");
    let payload = json!({
        "channel_id": CHANNEL_ID,
        "message": MESSAGE,
        "schedule_post_at": SEND_AT,
        "type": "TEXT",
        "props": {},
        "req_id": "g08-request"
    });
    harness.step(
        command_json("im_create_schedule", payload.clone()),
        SEND_AT as u64 - 100,
    );
    assert_eq!(
        harness.last_http_body(),
        Some(json!({
            "post": {
                "channelId": CHANNEL_ID,
                "message": MESSAGE,
                "type": "TEXT",
                "props": {}
            },
            "schedulePostAt": SEND_AT
        }))
    );
    let http_corr = correlation(harness.sink.as_slice(), "http")
        .ok_or_else(|| "missing createSchedule HTTP".to_string())?;
    harness.step(
        port_reply_with_body(
            http_corr.raw(),
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        SEND_AT as u64 - 99,
    );
    assert!(emitted_events(harness.sink.as_slice()).is_empty());

    let authority = created_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64 - 98,
    );
    assert_schedule_write_set(harness.sink.as_slice())?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing schedule PersistAtomic".to_string())?;
    harness.step(port_reply_ok(persist_corr.raw()), SEND_AT as u64 - 97);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "schedule PersistOk must request durable row".to_string())?;
    assert_eq!(
        emitted_events(harness.sink.as_slice()),
        vec![json!({
            "event": "im:read:result", "data": {"req_id": "g08-request", "body": {"ok": true}}
        })]
    );
    let row = durable_row();
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        SEND_AT as u64 - 96,
    );
    let events = emitted_events(harness.sink.as_slice());

    let duplicate = serde_json::to_vec(&created_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(duplicate))),
        SEND_AT as u64 - 95,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_create_schedule", "payload": payload},
                "now_ms": SEND_AT - 100
            }),
            json!({
                "kind": "Inbound",
                "value": authority,
                "now_ms": SEND_AT - 98
            }),
            json!({
                "kind": "PortReply",
                "value": {
                    "correlation": readback_corr.raw(),
                    "outcome": {"variant": "Ok", "rows": [{
                        "channel_id": CHANNEL_ID,
                        "schedule_id": SCHEDULE_ID,
                        "owner_user_id": CREATOR_ID,
                        "message": MESSAGE,
                        "message_preview": PREVIEW,
                        "send_at": SEND_AT,
                        "status": "scheduled",
                        "revision": REVISION
                    }]}
                },
                "now_ms": SEND_AT - 96
            }),
        ],
    })
}

/// 回放 observer 收到相同广播时的 creator-only no-op。
fn run_observer() -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[8], OBSERVER_ID, "g08-company", "", "", "");
    let authority = created_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(PersonaTrace {
        events: Vec::new(),
        ticks: vec![json!({
            "kind": "Inbound",
            "value": authority,
            "now_ms": SEND_AT
        })],
    })
}

/// 回放 creator 与 observer 两套独立本地状态。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_creator()?,
        receiver: run_observer()?,
    })
}

/// 构造 G-08 唯一 creator 业务终态。
fn expected_event() -> Value {
    json!({
        "event": "im:channel:schedule-created",
        "data": {
            "channelId": CHANNEL_ID,
            "hasSchedulePost": true,
            "scheduleId": SCHEDULE_ID,
            "ownerUserId": CREATOR_ID,
            "message": MESSAGE,
            "messagePreview": PREVIEW,
            "sendAt": SEND_AT,
            "status": "scheduled",
            "revision": REVISION
        }
    })
}

/// G-08 只向 creator 发布真实 schedule row，observer 保持空。
#[test]
fn g08_emits_creator_only_schedule_state() -> TestResult<()> {
    let trace = run_gate()?;
    assert_eq!(trace.sender.events, vec![expected_event()]);
    assert!(trace.receiver.events.is_empty());
    let encoded = serde_json::to_string(&trace.sender.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "actionCapabilities",
        "reqId",
        "operation",
        "scheduleMessage",
        "scheduleStatus",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// G-08 持久化失败不发布 creator 终态。
#[test]
fn g08_persist_failure_emits_no_terminal_event() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[8], CREATOR_ID, "g08-company", "", "", "");
    let inbound = serde_json::to_vec(&created_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing schedule PersistAtomic".to_string())?;
    harness.step(port_reply_err(persist_corr.raw()), SEND_AT as u64 + 1);
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    Ok(())
}

/// G-08 HTTP 失败只是 transport 失败，不建立 schedule。
#[test]
fn g08_http_failure_keeps_empty_schedule_state() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[8], CREATOR_ID, "g08-company", "", "", "");
    harness.step(
        command_json(
            "im_create_schedule",
            json!({
                "channel_id": CHANNEL_ID,
                "message": MESSAGE,
                "schedule_post_at": SEND_AT,
                "req_id": "g08-failed-request"
            }),
        ),
        SEND_AT as u64,
    );
    let http_corr = correlation(harness.sink.as_slice(), "http")
        .ok_or_else(|| "missing createSchedule HTTP".to_string())?;
    harness.step(port_reply_err(http_corr.raw()), SEND_AT as u64 + 1);
    assert_eq!(
        emitted_events(harness.sink.as_slice()),
        vec![json!({
            "event": "im:read:result", "data": {"req_id": "g08-failed-request", "error": "SCHEDULE_REQUEST_FAILED", "errorContext": format!("[command=im_create_schedule source=HTTP POST /posts/createSchedule stage=http req_id=g08-failed-request corr={}] SCHEDULE_REQUEST_FAILED; port failure: Http(500)", http_corr.raw())}
        })]
    );
    Ok(())
}

/// G-08 冷启动可从 durable channel_schedule row 重建相同终态。
#[test]
fn g08_cold_start_row_rebuilds_the_same_event() -> TestResult<()> {
    let event = helix_im::event::schedule::created_from_row(&durable_row())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "durable schedule row must rebuild an event".to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("schedule factory must return Emit".to_string());
    };
    let actual =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(actual, expected_event());
    Ok(())
}

/// Rust producer 在生成模式下写出 G-08 creator/observer canonical artifact。
#[test]
fn g08_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-08",
        "message_v3_g08",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post_schedule", "columns": ["channel_id", "schedule_id", "owner_user_id", "message", "message_preview", "send_at", "status", "revision"]}
        ]),
        json!([
            {"table": "channel_schedule", "columns": ["channel_id", "schedule_id", "owner_user_id", "message", "message_preview", "send_at", "status", "revision"]},
            {"table": "channel", "columns": ["id", "has_schedule_post", "schedule_id", "schedule_owner_user_id", "schedule_message", "schedule_message_preview", "schedule_send_at", "schedule_status", "schedule_revision"]}
        ]),
        json!([]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
