// 回归锚: Spec 047/T032-T033，G-04a 离线撤回重放与恢复幂等。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::{channel_id, Harness};

const CHANNEL_NUM: u64 = 4;
const CHANNEL_ID: &str = "chfixx00000000000000000004";
const POST_ID: &str = "srvrvka0000000000000000004";
const ACTOR_ID: &str = "g04a-actor";
const RECEIVER_ID: &str = "g04a-receiver";
const CREATE_AT: i64 = 1_700_000_304_000;

type TestResult<T> = Result<T, String>;

struct GateTrace {
    sender_events: Vec<Value>,
    receiver_events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 以固定字母表编码 host HTTP 信封，测试侧不新增运行时依赖。
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or_default());
        let b2 = u32::from(chunk.get(2).copied().unwrap_or_default());
        let value = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

/// 构造 public explicit sync command，平台 eventSeq 只作为不可信对照输入。
fn sync_command() -> TestResult<Tick> {
    helix_im::client_api::build_command(
        "im_sync_channels",
        json!({
            "channelId": CHANNEL_ID,
            "reason": "offline_revoke_replay",
            "eventSeq": 999_999
        })
        .to_string()
        .as_bytes(),
    )
    .map(Tick::Command)
    .map_err(|error| error.to_string())
}

/// 将真 Go type3 sync authority 包进 host ADR-007 HTTP 信封。
fn revoke_sync_reply() -> TestResult<ReplyBytes> {
    let body = json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": CHANNEL_ID,
                "kind": "events",
                "events": [{
                    "id": "g04a-revoke-event-1",
                    "channelId": CHANNEL_ID,
                    "eventSeq": 1,
                    "eventType": 3,
                    "msgId": POST_ID,
                    "actorId": ACTOR_ID,
                    "createAt": CREATE_AT,
                    "payload": {"reason": "offline_revoke_replay"}
                }],
                "messages": {},
                "nextSeq": 1,
                "resetTo": Value::Null
            }],
            "serverTime": CREATE_AT
        }
    });
    http_reply(body)
}

/// 构造恢复 cursor 已追平时的 no_change HTTP 信封。
fn no_change_sync_reply() -> TestResult<ReplyBytes> {
    http_reply(json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": CHANNEL_ID,
                "kind": "no_change",
                "events": [],
                "nextSeq": 1,
                "resetTo": Value::Null
            }],
            "serverTime": CREATE_AT + 1
        }
    }))
}

/// 把 Go body 编码为 host driver 回灌的 ReplyBytes。
fn http_reply(body: Value) -> TestResult<ReplyBytes> {
    let raw = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let envelope = serde_json::to_vec(&json!({
        "status": 200,
        "headers": {},
        "body": base64(&raw),
    }))
    .map_err(|error| error.to_string())?;
    Ok(ReplyBytes(Bytes::from(envelope)))
}

/// 从当前 Effect batch 读取唯一 HTTP correlation。
fn http_correlation(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::Http { corr, .. } => Some(*corr),
        _ => None,
    })
}

/// 从当前 Effect batch 读取唯一原子持久 correlation。
fn atomic_correlation(effects: &[Effect]) -> Option<Correlation> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, .. } => Some(*corr),
        _ => None,
    })
}

/// 解出当前 batch 的全部 MessageV3 事件。
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

/// 断言 type3 与 cursor 同批持久，且 message 只做幂等 revoke patch。
fn assert_revoke_commit(effects: &[Effect]) -> TestResult<()> {
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .ok_or_else(|| "G-04a authority must persist atomically".to_string())?;
    let revoked = ops.iter().any(|op| match op {
        StorageOp::BatchUpdate(spec) if spec.table == "message" => {
            matches!(
                spec.key_vals.as_slice(),
                [SqlValue::Text(id)] if id == POST_ID
            ) && spec
                .patch
                .iter()
                .any(|(column, value)| column == "revoke" && matches!(value, SqlValue::Integer(1)))
        }
        _ => false,
    });
    let cursor = ops.iter().any(|op| match op {
        StorageOp::MonotonicUpsert(spec) => spec.value == 1,
        _ => false,
    });
    if !revoked || !cursor {
        return Err(format!(
            "G-04a commit missing revoke/cursor: revoked={revoked}, cursor={cursor}"
        ));
    }
    Ok(())
}

/// 回放离线 receiver 的首次 sync、重复 replay 与冷启动 no_change。
fn run_gate() -> TestResult<GateTrace> {
    let mut receiver =
        Harness::new_with_identity(&[CHANNEL_NUM], RECEIVER_ID, "g04a-company", "", "", "");
    let mut ticks = Vec::new();

    receiver.step(sync_command()?, CREATE_AT as u64);
    let http_corr = http_correlation(receiver.sink.as_slice())
        .ok_or_else(|| "G-04a explicit sync missing HTTP".to_string())?;
    ticks.push(json!({"kind": "Command", "value": {"name": "im_sync_channels", "channelId": CHANNEL_ID, "reason": "offline_revoke_replay"}, "now_ms": CREATE_AT}));
    if !emitted_events(receiver.sink.as_slice()).is_empty() {
        return Err("sync admission emitted a terminal event".to_string());
    }

    receiver.step(
        Tick::PortReply {
            corr: http_corr,
            outcome: PortOutcome::Ok(revoke_sync_reply()?),
        },
        CREATE_AT as u64 + 1,
    );
    assert_revoke_commit(receiver.sink.as_slice())?;
    let persist_corr = atomic_correlation(receiver.sink.as_slice())
        .ok_or_else(|| "G-04a sync missing atomic persist".to_string())?;
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": http_corr.raw(), "outcome": {"variant": "Ok", "authority": {"eventType": 3, "eventSeq": 1, "msgId": POST_ID, "actorId": ACTOR_ID}}}, "now_ms": CREATE_AT + 1}));
    if !emitted_events(receiver.sink.as_slice()).is_empty() {
        return Err("sync authority emitted before PersistOk".to_string());
    }

    receiver.step(
        Tick::PortReply {
            corr: persist_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        CREATE_AT as u64 + 2,
    );
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": persist_corr.raw(), "outcome": {"variant": "Ok"}}, "now_ms": CREATE_AT + 2}));
    let receiver_events = emitted_events(receiver.sink.as_slice());

    receiver.step(sync_command()?, CREATE_AT as u64 + 3);
    ticks.push(json!({"kind": "Command", "value": {"name": "im_sync_channels", "channelId": CHANNEL_ID, "reason": "duplicate_replay"}, "now_ms": CREATE_AT + 3}));
    let replay_corr = http_correlation(receiver.sink.as_slice())
        .ok_or_else(|| "G-04a replay missing HTTP".to_string())?;
    receiver.step(
        Tick::PortReply {
            corr: replay_corr,
            outcome: PortOutcome::Ok(revoke_sync_reply()?),
        },
        CREATE_AT as u64 + 4,
    );
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": replay_corr.raw(), "outcome": {"variant": "Ok", "authority": {"eventType": 3, "eventSeq": 1, "msgId": POST_ID, "duplicate": true}}}, "now_ms": CREATE_AT + 4}));
    if emitted_events(receiver.sink.as_slice())
        .iter()
        .any(|event| event["event"] == "im:post:revoke")
    {
        return Err("stale replay emitted duplicate revoke".to_string());
    }

    let mut recovered = Harness::new_with_identity(&[], RECEIVER_ID, "g04a-company", "", "", "");
    recovered
        .module
        .register_channel(channel_id(CHANNEL_NUM), 1);
    recovered.step(sync_command()?, CREATE_AT as u64 + 5);
    ticks.push(json!({"kind": "Command", "value": {"name": "im_sync_channels", "channelId": CHANNEL_ID, "reason": "cold_start_recovery", "restoredCursor": 1}, "now_ms": CREATE_AT + 5}));
    let recovery_corr = http_correlation(recovered.sink.as_slice())
        .ok_or_else(|| "G-04a recovery missing HTTP".to_string())?;
    recovered.step(
        Tick::PortReply {
            corr: recovery_corr,
            outcome: PortOutcome::Ok(no_change_sync_reply()?),
        },
        CREATE_AT as u64 + 6,
    );
    ticks.push(json!({"kind": "PortReply", "value": {"correlation": recovery_corr.raw(), "outcome": {"variant": "Ok", "authority": {"kind": "no_change", "nextSeq": 1, "restoredCursor": 1}}}, "now_ms": CREATE_AT + 6}));
    if !emitted_events(recovered.sink.as_slice()).is_empty() {
        return Err("cold-start no_change emitted duplicate terminal state".to_string());
    }

    Ok(GateTrace {
        sender_events: Vec::new(),
        receiver_events,
        ticks,
    })
}

/// G-04a 只让离线 receiver 在提交后看到最小 revoke，并保持 replay/recovery 幂等。
#[test]
fn g04a_replays_revoke_once_after_persist_and_recovers_idempotently() -> TestResult<()> {
    let trace = run_gate()?;
    if !trace.sender_events.is_empty() {
        return Err("online sender must not receive offline replay again".to_string());
    }
    let names = trace
        .receiver_events
        .iter()
        .filter_map(|event| event["event"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["im:post:revoke", "im:sync:state"]);
    let revoked = &trace.receiver_events[0]["data"];
    assert_eq!(revoked["id"], POST_ID);
    assert_eq!(revoked["channelId"], CHANNEL_ID);
    assert_eq!(revoked["eventSeq"], 1);
    assert_eq!(revoked["revoke"], true);
    for forbidden in [
        "actorId",
        "actorName",
        "displayText",
        "systemEvent",
        "targetMsgId",
    ] {
        assert!(
            revoked.get(forbidden).is_none(),
            "forbidden field {forbidden}"
        );
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-04a sender/receiver canonical artifact。
#[test]
fn g04a_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_gate()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-04a",
        "message_v3_g04a",
        trace.ticks,
        trace.sender_events,
        trace.receiver_events,
        json!([
            {"table": "channel_event", "columns": ["id", "channel_id", "event_seq", "event_type", "msg_id", "actor_id", "create_at", "payload"]},
            {"table": "post", "columns": ["id", "revoke"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "revoke"]},
            {"table": "_helix_monotonic", "columns": ["scope", "key", "value"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "revoke"]},
            {"table": "_helix_monotonic", "columns": ["scope", "key", "value"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
