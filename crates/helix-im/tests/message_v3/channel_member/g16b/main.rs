// 回归锚: Spec 047/T070-T071，G-16b 成员离场双视角。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

const CHANNEL_ID: &str = "chfixx0000000000000000001b";
const LEAVER_ID: &str = "g16b-leaver";
type TestResult<T> = Result<T, String>;

/// 用给定 viewer 回放同一成员离场 authority。
fn run_persona(viewer: &str) -> TestResult<(Vec<Value>, u64)> {
    let mut harness = common::Harness::new_with_identity(&[], viewer, "g16b-company", "", "", "");
    let inbound = serde_json::to_vec(&json!({
        "action":"channel_member_update",
        "data":{
            "id":CHANNEL_ID,
            "type":"P",
            "event_seq":51,
            "memberChange":{"leave":[{"id":LEAVER_ID}]}
        }
    }))
    .map_err(|error| error.to_string())?;
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound))), 1_700_001_610_000);
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } | Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing member persistence".to_string())?;
    harness.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_610_001,
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

/// G-16b leaver 与 remaining member 都收到 roster 更新和离场事实。
#[test]
fn g16b_emits_roster_and_leave_for_both_personas() -> TestResult<()> {
    for viewer in [LEAVER_ID, "g16b-remaining"] {
        let (events, _) = run_persona(viewer)?;
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event"], "im:channel-member:updated");
        assert_eq!(events[0]["data"]["leaves"][0], LEAVER_ID);
        assert_eq!(events[1]["event"], "im:channel-member:left");
        assert_eq!(events[1]["data"]["userId"], LEAVER_ID);
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-16b 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let (sender, corr) = run_persona(LEAVER_ID)?;
    let (receiver, _) = run_persona("g16b-remaining")?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else { return Ok(()); };
    let storage = json!([{"table":"channel_member","columns":["channel_id","user_id","role","nick_name"]}]);
    let bundle = artifact::gate_bundle(
        "G-16b",
        "message_v3_g16b",
        vec![
            json!({"kind":"Inbound","value":{"action":"channel_member_update","data":{"id":CHANNEL_ID,"event_seq":51,"memberChange":{"leave":[{"id":LEAVER_ID}]}}},"now_ms":1_700_001_610_000_i64}),
            json!({"kind":"PortReply","value":{"correlation":corr,"outcome":{"variant":"Ok"}},"now_ms":1_700_001_610_001_i64})
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
