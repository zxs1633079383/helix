// 回归锚: Spec 047/T046-T047，G-09 creator-only schedule cancel durable 绝对态。

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

const CHANNEL_ID: &str = "chfixx00000000000000000009";
const SCHEDULE_ID: &str = "schedule-g09-1";
const CREATOR_ID: &str = "g09-creator";
const OBSERVER_ID: &str = "g09-observer";
const MESSAGE: &str = "G09 scheduled message";
const PREVIEW: &str = "G09 scheduled preview";
const SEND_AT: i64 = 1_700_000_960_000;
const CREATED_REVISION: u64 = 8;
const CANCELED_REVISION: u64 = 9;

type TestResult<T> = Result<T, String>;

/// 保存 creator 或 observer 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-09 双 persona 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造 G-09 前置 `post_schedule_created` 权威帧。
fn created_authority() -> Value {
    json!({
        "action": "post_schedule_created",
        "eventSeq": CREATED_REVISION,
        "data": {
            "channelId": CHANNEL_ID,
            "scheduleId": SCHEDULE_ID,
            "scheduleOwnerUserId": CREATOR_ID,
            "message": MESSAGE,
            "messagePreview": PREVIEW,
            "sendAt": SEND_AT,
            "status": "scheduled",
            "revision": CREATED_REVISION
        }
    })
}

/// 构造 Go `post_schedule_canceled` creator authority 帧。
fn canceled_authority() -> Value {
    json!({
        "action": "post_schedule_canceled",
        "eventSeq": CANCELED_REVISION,
        "data": {
            "channelId": CHANNEL_ID,
            "scheduleOwnerUserId": CREATOR_ID,
            "status": "canceled",
            "revision": CANCELED_REVISION
        }
    })
}

/// 构造 creator 本地已排程 channel_schedule 读回行。
fn scheduled_row() -> Row {
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
        (
            "revision".to_string(),
            SqlValue::Integer(CREATED_REVISION as i64),
        ),
    ]
}

/// 构造 creator 本地取消后的 channel_schedule 读回行。
fn canceled_row() -> Row {
    vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(CHANNEL_ID.to_string()),
        ),
        ("schedule_id".to_string(), SqlValue::Text(String::new())),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(CREATOR_ID.to_string()),
        ),
        ("message".to_string(), SqlValue::Text(String::new())),
        ("message_preview".to_string(), SqlValue::Text(String::new())),
        ("send_at".to_string(), SqlValue::Integer(0)),
        ("status".to_string(), SqlValue::Text("canceled".to_string())),
        (
            "revision".to_string(),
            SqlValue::Integer(CANCELED_REVISION as i64),
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

/// 断言 G-09 原子清空 schedule 正文并同步 channel 终态。
fn assert_cancel_write_set(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-09 authority must persist atomically".to_string())?;
    assert_eq!(ops.len(), 2);
    let schedule_rows = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == "channel_schedule" => Some(&spec.rows),
        _ => None,
    });
    let schedule_rows =
        schedule_rows.ok_or_else(|| "G-09 must update channel_schedule".to_string())?;
    let row = schedule_rows
        .first()
        .ok_or_else(|| "G-09 schedule upsert requires one row".to_string())?;
    assert!(row.iter().any(|(column, value)| {
        column == "message" && matches!(value, SqlValue::Text(value) if value.is_empty())
    }));
    assert!(row.iter().any(|(column, value)| {
        column == "status" && matches!(value, SqlValue::Text(value) if value == "canceled")
    }));
    let channel_patch = ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(&spec.patch),
        _ => None,
    });
    let channel_patch =
        channel_patch.ok_or_else(|| "G-09 must update channel read model".to_string())?;
    assert!(channel_patch.iter().any(|(column, value)| {
        column == "has_schedule_post" && matches!(value, SqlValue::Integer(0))
    }));
    assert!(channel_patch.iter().any(|(column, value)| {
        column == "schedule_revision"
            && matches!(value, SqlValue::Integer(value) if *value == CANCELED_REVISION as i64)
    }));
    Ok(())
}

/// 建立 creator 的 G-08 committed prerequisite，供 G-09 单调性验证。
fn commit_scheduled_prerequisite(harness: &mut Harness) -> TestResult<()> {
    let inbound = serde_json::to_vec(&created_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64 - 10,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing G-08 prerequisite PersistAtomic".to_string())?;
    harness.step(port_reply_ok(persist_corr.raw()), SEND_AT as u64 - 9);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing G-08 prerequisite readback".to_string())?;
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&scheduled_row())),
        ),
        SEND_AT as u64 - 8,
    );
    assert_eq!(emitted_events(harness.sink.as_slice()).len(), 1);
    Ok(())
}

/// 回放 creator cancel command、WS authority、PersistOk 与 durable readback。
fn run_creator() -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[9], CREATOR_ID, "g09-company", "", "", "");
    commit_scheduled_prerequisite(&mut harness)?;
    let payload = json!({
        "channel_id": CHANNEL_ID,
        "req_id": "g09-cancel-request"
    });
    harness.step(
        command_json("im_cancel_schedule", payload.clone()),
        SEND_AT as u64,
    );
    assert_eq!(
        harness.last_http_body(),
        Some(json!({"channelId": CHANNEL_ID}))
    );
    let http_corr = correlation(harness.sink.as_slice(), "http")
        .ok_or_else(|| "missing cancelSchedule HTTP".to_string())?;
    harness.step(
        port_reply_with_body(
            http_corr.raw(),
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        SEND_AT as u64 + 1,
    );
    assert!(emitted_events(harness.sink.as_slice()).is_empty());

    let authority = canceled_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64 + 2,
    );
    assert_cancel_write_set(harness.sink.as_slice())?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing G-09 PersistAtomic".to_string())?;
    harness.step(port_reply_ok(persist_corr.raw()), SEND_AT as u64 + 3);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "G-09 PersistOk must request durable row".to_string())?;
    assert_eq!(
        emitted_events(harness.sink.as_slice()),
        vec![json!({
            "event": "im:read:result", "data": {"req_id": "g09-cancel-request", "body": {"ok": true}}
        })]
    );
    let row = canceled_row();
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        SEND_AT as u64 + 4,
    );
    let events = emitted_events(harness.sink.as_slice());
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({
                "kind": "Inbound",
                "value": created_authority(),
                "now_ms": SEND_AT - 10
            }),
            json!({
                "kind": "Command",
                "value": {"name": "im_cancel_schedule", "payload": payload},
                "now_ms": SEND_AT
            }),
            json!({
                "kind": "Inbound",
                "value": authority,
                "now_ms": SEND_AT + 2
            }),
            json!({
                "kind": "PortReply",
                "value": {
                    "correlation": readback_corr.raw(),
                    "outcome": {"variant": "Ok", "rows": [{
                        "channel_id": CHANNEL_ID,
                        "schedule_id": "",
                        "owner_user_id": CREATOR_ID,
                        "message": "",
                        "message_preview": "",
                        "send_at": 0,
                        "status": "canceled",
                        "revision": CANCELED_REVISION
                    }]}
                },
                "now_ms": SEND_AT + 4
            }),
        ],
    })
}

/// 回放 observer 收到 creator-private cancel 广播时的 no-op。
fn run_observer() -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[9], OBSERVER_ID, "g09-company", "", "", "");
    let authority = canceled_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        SEND_AT as u64 + 5,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(PersonaTrace {
        events: Vec::new(),
        ticks: vec![json!({
            "kind": "Inbound",
            "value": authority,
            "now_ms": SEND_AT + 5
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

/// 构造 G-09 唯一 creator 业务终态。
fn expected_event() -> Value {
    json!({
        "event": "im:channel:schedule-canceled",
        "data": {
            "channelId": CHANNEL_ID,
            "hasSchedulePost": false,
            "status": "canceled",
            "revision": CANCELED_REVISION
        }
    })
}

/// G-09 只向 creator 发布取消绝对态，observer 保持空。
#[test]
fn g09_emits_creator_only_canceled_state() -> TestResult<()> {
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

/// G-09 持久化失败不发布 creator 取消终态。
#[test]
fn g09_persist_failure_emits_no_terminal_event() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[9], CREATOR_ID, "g09-company", "", "", "");
    commit_scheduled_prerequisite(&mut harness)?;
    harness.step(
        command_json(
            "im_cancel_schedule",
            json!({"channel_id": CHANNEL_ID, "req_id": "g09-failed-request"}),
        ),
        SEND_AT as u64,
    );
    let authority = serde_json::to_vec(&canceled_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(authority))),
        SEND_AT as u64 + 1,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing G-09 PersistAtomic".to_string())?;
    harness.step(port_reply_err(persist_corr.raw()), SEND_AT as u64 + 2);
    assert_eq!(
        emitted_events(harness.sink.as_slice()),
        vec![json!({
            "event": "im:read:result", "data": {"req_id": "g09-failed-request", "error": "SCHEDULE_PERSIST_FAILED"}
        })]
    );
    Ok(())
}

/// G-09 已提交 revision 阻止迟到 G-08 重建旧 schedule。
#[test]
fn g09_late_create_cannot_resurrect_canceled_schedule() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[9], CREATOR_ID, "g09-company", "", "", "");
    commit_scheduled_prerequisite(&mut harness)?;
    harness.step(
        command_json(
            "im_cancel_schedule",
            json!({"channel_id": CHANNEL_ID, "req_id": "g09-race-request"}),
        ),
        SEND_AT as u64,
    );
    let cancel = serde_json::to_vec(&canceled_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(cancel))),
        SEND_AT as u64 + 1,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "missing G-09 PersistAtomic".to_string())?;
    harness.step(port_reply_ok(persist_corr.raw()), SEND_AT as u64 + 2);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing G-09 readback".to_string())?;
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        SEND_AT as u64 + 3,
    );
    assert_eq!(
        emitted_events(harness.sink.as_slice()),
        vec![expected_event()]
    );

    let late = serde_json::to_vec(&created_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(late))),
        SEND_AT as u64 + 4,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(())
}

/// G-09 冷启动可从 durable channel_schedule row 重建相同取消终态。
#[test]
fn g09_cold_start_row_rebuilds_the_same_event() -> TestResult<()> {
    let event = helix_im::event::schedule::canceled_from_row(&canceled_row())
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "durable canceled row must rebuild an event".to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("schedule factory must return Emit".to_string());
    };
    let actual =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(actual, expected_event());
    Ok(())
}

/// Rust producer 在生成模式下写出 G-09 creator/observer canonical artifact。
#[test]
fn g09_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-09",
        "message_v3_g09",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post_schedule", "columns": ["channel_id", "owner_user_id", "status", "revision"]}
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
