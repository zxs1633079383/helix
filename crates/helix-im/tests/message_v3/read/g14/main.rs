// 回归锚: Spec 047/T040-T041，G-14 viewer-relative read/readers 终态。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::{command_json, port_reply_err, port_reply_ok, Harness};

const CHANNEL_ID: &str = "chfixx0000000000000000000e";
const POST_ID: &str = "srvrea00000000000000000014";
const READER_ID: &str = "g14-reader";
const AUTHOR_ID: &str = "g14-author";
const READ_BITS: &str = "010";
const REVISION: i64 = 1_700_001_400_020;

type TestResult<T> = Result<T, String>;

/// 保存一个 persona 的真实输入序列和业务出站。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 保存 G-14 reader/author 的独立本地状态迁移。
struct GateTrace {
    sender: PersonaTrace,
    receiver: PersonaTrace,
}

/// 构造 Go 无 event_seq 的 `post_read` 权威帧。
fn read_authority() -> Value {
    json!({
        "action": "post_read",
        "data": {
            "channelId": CHANNEL_ID,
            "postId": POST_ID,
            "userId": READER_ID,
            "readMap": READ_BITS,
            "createAt": REVISION - 20,
            "updateAt": REVISION
        }
    })
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
        ("persist", Effect::Persist { corr, .. }) => Some(*corr),
        _ => None,
    })
}

/// 断言 G-14 只覆盖 message.read_bits，不推进 channel cursor。
fn assert_read_write_set(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-14 authority must persist read_bits".to_string())?;
    assert_eq!(ops.len(), 1);
    let Some(StorageOp::BatchUpdate(spec)) = ops.first() else {
        return Err("G-14 write must be a BatchUpdate".to_string());
    };
    assert_eq!(spec.table, "message");
    assert_eq!(spec.key_col, "id");
    assert!(spec.patch.iter().any(|(column, value)| {
        column == "read_bits" && matches!(value, SqlValue::Text(value) if value == READ_BITS)
    }));
    assert!(ops.iter().all(|op| !matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
    Ok(())
}

/// 回放一个 viewer 的 command 可选项、WS authority 与 PersistOk。
fn run_persona(viewer_id: &str, include_command: bool) -> TestResult<PersonaTrace> {
    let mut harness = Harness::new_with_identity(&[14], viewer_id, "g14-company", "", "", "");
    let mut ticks = Vec::new();
    if include_command {
        let payload = json!({"channel_id": CHANNEL_ID, "posts": [POST_ID]});
        harness.step(
            command_json("im_post_read", payload.clone()),
            REVISION as u64,
        );
        assert_eq!(
            harness.last_http_body(),
            Some(json!({"channelId": CHANNEL_ID, "posts": [POST_ID]}))
        );
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "missing post/read HTTP".to_string())?;
        harness.step(port_reply_ok(http_corr.raw()), REVISION as u64 + 1);
        assert!(
            emitted_events(harness.sink.as_slice()).is_empty(),
            "HTTP ack is admission only"
        );
        ticks.push(json!({
            "kind": "Command",
            "value": {"name": "im_post_read", "payload": payload},
            "now_ms": REVISION
        }));
    }

    let authority = read_authority();
    let inbound = serde_json::to_vec(&authority).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        REVISION as u64 + 2,
    );
    assert_read_write_set(harness.sink.as_slice())?;
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let persist_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing post_read Persist".to_string())?;
    ticks.push(json!({
        "kind": "Inbound",
        "value": authority,
        "now_ms": REVISION + 2
    }));

    harness.step(port_reply_ok(persist_corr.raw()), REVISION as u64 + 3);
    let events = emitted_events(harness.sink.as_slice());

    let duplicate = serde_json::to_vec(&read_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(duplicate))),
        REVISION as u64 + 4,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same read authority must be idempotent"
    );
    Ok(PersonaTrace { events, ticks })
}

/// 回放 reader 与 author 两套独立本地状态。
fn run_gate() -> TestResult<GateTrace> {
    Ok(GateTrace {
        sender: run_persona(READER_ID, true)?,
        receiver: run_persona(AUTHOR_ID, false)?,
    })
}

/// G-14 reader 只收到 read，author 只收到 readers。
#[test]
fn g14_splits_reader_and_author_events_without_frame_refresh() -> TestResult<()> {
    let trace = run_gate()?;
    assert_eq!(
        trace.sender.events,
        vec![json!({
            "event": "im:post:read",
            "data": {
                "id": POST_ID,
                "channelId": CHANNEL_ID,
                "readBits": READ_BITS,
                "readerId": READER_ID,
                "receiptRevision": REVISION
            }
        })]
    );
    assert_eq!(
        trace.receiver.events,
        vec![json!({
            "event": "im:post:readers",
            "data": {
                "id": POST_ID,
                "channelId": CHANNEL_ID,
                "readBits": READ_BITS,
                "readerIds": [READER_ID],
                "receiptRevision": REVISION
            }
        })]
    );
    let encoded =
        serde_json::to_string(&trace.receiver.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "actionCapabilities",
        "receipts",
        "reqId",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// G-14 持久化失败不发布 reader 或 author 终态。
#[test]
fn g14_persist_failure_emits_no_terminal_event() -> TestResult<()> {
    let mut harness = Harness::new_with_identity(&[14], AUTHOR_ID, "g14-company", "", "", "");
    let inbound = serde_json::to_vec(&read_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        REVISION as u64,
    );
    let persist_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "missing post_read Persist".to_string())?;
    harness.step(port_reply_err(persist_corr.raw()), REVISION as u64 + 1);
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    Ok(())
}

/// G-14 durable read_bits 与恢复时已知 reader authority 可重建相同 author 终态。
#[test]
fn g14_recovery_rebuilds_author_event_from_committed_facts() -> TestResult<()> {
    let event = helix_im::event::read::post_for_viewer(
        CHANNEL_ID, POST_ID, READER_ID, READ_BITS, REVISION, AUTHOR_ID,
    )
    .map_err(|error| error.to_string())?;
    let Effect::Emit { event } = event.into_effect() else {
        return Err("read factory must return Emit".to_string());
    };
    let actual =
        serde_json::from_slice::<Value>(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(actual["event"], "im:post:readers");
    assert_eq!(actual["data"]["readBits"], READ_BITS);
    assert_eq!(actual["data"]["readerIds"], json!([READER_ID]));
    Ok(())
}

/// Rust producer 在生成模式下写出 G-14 reader/author canonical artifact。
#[test]
fn g14_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let mut ticks = trace.sender.ticks;
    ticks.extend(trace.receiver.ticks);
    let bundle = artifact::gate_bundle(
        "G-14",
        "message_v3_g14",
        ticks,
        trace.sender.events,
        trace.receiver.events,
        json!([
            {"table": "post", "columns": ["id", "channel_id", "read_bits", "update_at"]},
            {"table": "channel_event", "columns": ["msg_id", "actor_id", "event_type"]}
        ]),
        json!([{"table": "message", "columns": ["id", "channel_id", "read_bits"]}]),
        json!([{"table": "message", "columns": ["id", "channel_id", "read_bits"]}]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
