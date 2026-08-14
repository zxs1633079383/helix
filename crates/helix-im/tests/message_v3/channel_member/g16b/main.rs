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

/// 只为仍在册的 receiver 回放 Go 单播的成员变更 authority。
fn run_remaining_receiver() -> TestResult<(Vec<Value>, u64)> {
    let viewer = "g16b-remaining";
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
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        1_700_001_610_000,
    );
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
    let channel_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing channel readback".to_string())?;
    harness.step(
        Tick::PortReply {
            corr: channel_corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                serde_json::to_vec(&[json!({"id":CHANNEL_ID,"type":"P"})])
                    .map_err(|error| error.to_string())?,
            ))),
        },
        1_700_001_610_002,
    );
    let roster_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing roster readback".to_string())?;
    harness.step(
        Tick::PortReply {
            corr: roster_corr,
            outcome: PortOutcome::Ok(ReplyBytes(Bytes::from(
                serde_json::to_vec(&[json!({
                    "channel_id":CHANNEL_ID,
                    "user_id":viewer,
                    "team_id":"g16b-company",
                    "role":"MEMBER",
                    "nick_name":""
                })])
                .map_err(|error| error.to_string())?,
            ))),
        },
        1_700_001_610_003,
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

/// G-16b 只向 Go 仍在册的 receiver 交付唯一 canonical 成员 roster。
#[test]
fn g16b_emits_one_canonical_roster_for_remaining_receiver() -> TestResult<()> {
    let (events, _) = run_remaining_receiver()?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:channel:member-updated");
    assert_eq!(events[0]["data"]["channel"]["memberCount"], 1);
    assert!(events[0]["data"]["channel"]
        .to_string()
        .find(LEAVER_ID)
        .is_none());
    Ok(())
}

/// Rust producer 在生成模式下写出 G-16b 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let (receiver, corr) = run_remaining_receiver()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let storage =
        json!([{"table":"channel_member","columns":["channel_id","user_id","role","nick_name"]}]);
    let bundle = artifact::gate_bundle(
        "G-16b",
        "message_v3_g16b",
        vec![
            json!({"kind":"Inbound","value":{"action":"channel_member_update","data":{"id":CHANNEL_ID,"event_seq":51,"memberChange":{"leave":[{"id":LEAVER_ID}]}}},"now_ms":1_700_001_610_000_i64}),
            json!({"kind":"PortReply","value":{"correlation":corr,"outcome":{"variant":"Ok"}},"now_ms":1_700_001_610_001_i64}),
        ],
        // Go 在 soft-delete 后只把该 WS frame 单播给 remaining members；离开者走 G08d lifecycle。
        vec![],
        receiver,
        storage.clone(),
        storage.clone(),
        storage,
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
