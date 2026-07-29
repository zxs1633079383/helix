// 回归锚: Spec 047/T022-T023，G-15a 双 persona 建群与持久屏障。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

use common::Harness;

const CHANNEL_ID: &str = "chfixx00000000000000000015";
const ACTOR_ID: &str = "g15a-actor";
const OBSERVER_ID: &str = "g15a-observer";

struct PersonaTrace {
    events: Vec<Value>,
    inbound: Value,
    persist_corr: u64,
}

type TestResult<T> = Result<T, String>;

/// 构造 HTTP/WS 共用的新频道权威对象。
fn authority() -> Value {
    json!({
        "id": CHANNEL_ID,
        "type": "O",
        "displayName": "G15a MessageV3 Group",
        "teamId": "g15a-company",
        "createAt": 1_700_001_500_000_i64,
        "members": [
            {"id": OBSERVER_ID, "role": "MEMBER", "nickName": "Observer"}
        ],
        "owner": {"id": ACTOR_ID, "role": "OWNER", "nickName": "Actor"}
    })
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

/// 返回 PersistAtomic 写集覆盖的表名，证明 channel 与 roster 同一提交。
fn persisted_tables(effect: &Effect) -> Option<BTreeSet<&'static str>> {
    let Effect::PersistAtomic { ops, .. } = effect else {
        return None;
    };
    Some(
        ops.iter()
            .filter_map(|op| match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => Some(spec.table),
                _ => None,
            })
            .collect(),
    )
}

/// 用独立 auth identity 回放同一 authority，得到该 persona 的提交后事件。
fn run_persona(auth_user_id: &str) -> TestResult<PersonaTrace> {
    let inbound = json!({
        "action": "channel_created",
        "data": authority()
    });
    let bytes = serde_json::to_vec(&inbound).map_err(|error| error.to_string())?;
    let mut harness = Harness::new_with_identity(&[], auth_user_id, "g15a-company", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(bytes))),
        1_700_001_500_000,
    );
    assert_eq!(harness.sink.as_slice().len(), 1);
    let first_effect = harness
        .sink
        .as_slice()
        .first()
        .ok_or_else(|| "missing G-15a persistence effect".to_string())?;
    let tables = persisted_tables(first_effect)
        .ok_or_else(|| "G-15a must start with PersistAtomic".to_string())?;
    assert!(tables.contains("channel"));
    assert!(tables.contains("channel_member"));
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing G-15a persistence correlation".to_string())?;

    harness.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_500_001,
    );
    Ok(PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        inbound,
        persist_corr: corr.raw(),
    })
}

/// G-15a sender/receiver 共享频道 identity，但 viewer role 必须独立计算。
#[test]
fn g15a_emits_committed_channel_and_roster_for_both_personas() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID)?;
    let receiver = run_persona(OBSERVER_ID)?;
    for trace in [&sender, &receiver] {
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[0]["event"], "im:channel:created");
        assert_eq!(trace.events[1]["event"], "im:channel:members");
        assert_eq!(trace.events[0]["data"]["channelId"], CHANNEL_ID);
        assert_eq!(trace.events[0]["data"]["memberCount"], 2);
        let members = trace.events[1]["data"]["members"]
            .as_array()
            .ok_or_else(|| "members must be an array".to_string())?;
        assert_eq!(members.len(), 2);
        let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
        assert!(!encoded.contains("InteractionFrame"));
        assert!(!encoded.contains("Surface"));
        assert!(!encoded.contains("actionCapabilities"));
    }
    assert_eq!(sender.events[0]["data"]["viewerRole"], "OWNER");
    assert_eq!(receiver.events[0]["data"]["viewerRole"], "MEMBER");
    Ok(())
}

/// Rust producer 在生成模式下写出 G-15a 双视角 canonical artifact。
#[test]
fn g15a_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID)?;
    let receiver = run_persona(OBSERVER_ID)?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-15a",
        "message_v3_g15a",
        vec![
            json!({
                "kind": "Inbound",
                "value": sender.inbound,
                "now_ms": 1_700_001_500_000_u64
            }),
            json!({
                "kind": "PortReply",
                "value": {
                    "correlation": sender.persist_corr,
                    "outcome": {"variant": "Ok", "reply_bytes": ""}
                },
                "now_ms": 1_700_001_500_001_u64
            }),
        ],
        sender.events,
        receiver.events,
        json!([
            {"table": "channel", "columns": ["id", "display_name", "type", "owner", "create_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "role", "nick_name"]}
        ]),
        json!([
            {"table": "channel", "columns": ["id", "display_name", "type", "owner", "created_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "role", "nick_name"]}
        ]),
        json!([
            {"table": "channel", "columns": ["id", "display_name", "type", "owner", "created_at"]},
            {"table": "channel_member", "columns": ["channel_id", "user_id", "role", "nick_name"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())?;
    Ok(())
}
