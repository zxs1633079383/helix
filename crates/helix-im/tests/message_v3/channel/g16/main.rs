// 回归锚: Spec 047/T068-T069，G-16 频道关闭生命周期。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

const CHANNEL_ID: &str = "chfixx00000000000000000016";
type TestResult<T> = Result<T, String>;

/// 通过真实 WS handler 和 PersistAtomic 回执生成关闭终态。
fn run_persona(user_id: &str) -> TestResult<(Vec<Value>, u64)> {
    let mut harness = common::Harness::new_with_identity(&[], user_id, "g16-company", "", "", "");
    let inbound = serde_json::to_vec(&json!({
        "action":"channel_close",
        "data":{"channelId":CHANNEL_ID,"deleteAt":1_700_001_600_010_i64}
    }))
    .map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        1_700_001_600_010,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing lifecycle persistence".to_string())?;
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    harness.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_600_011,
    );
    let events = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect();
    Ok((events, corr.raw()))
}

/// G-16 actor 与 observer 都在持久屏障后收到关闭绝对态。
#[test]
fn g16_emits_committed_lifecycle_for_both_personas() -> TestResult<()> {
    for user_id in ["g16-actor", "g16-observer"] {
        let (events, _) = run_persona(user_id)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "im:channel:closed");
        assert_eq!(events[0]["data"]["channelId"], CHANNEL_ID);
        assert_eq!(events[0]["data"]["closed"], true);
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-16 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let (sender, corr) = run_persona("g16-actor")?;
    let (receiver, _) = run_persona("g16-observer")?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let storage = json!([{"table":"channel","columns":["id","delete_at"]}]);
    let bundle = artifact::gate_bundle(
        "G-16",
        "message_v3_g16",
        vec![
            json!({"kind":"Inbound","value":{"action":"channel_close","data":{"channelId":CHANNEL_ID,"deleteAt":1_700_001_600_010_i64}},"now_ms":1_700_001_600_010_i64}),
            json!({"kind":"PortReply","value":{"correlation":corr,"outcome":{"variant":"Ok"}},"now_ms":1_700_001_600_011_i64}),
        ],
        sender,
        receiver,
        storage.clone(),
        storage.clone(),
        storage,
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
