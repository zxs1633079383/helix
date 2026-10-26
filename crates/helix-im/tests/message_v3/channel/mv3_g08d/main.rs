#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

const GATE_ID: &str = "MV3-G08d";

/// 锁定 G08d 的完整只读参考包，供后续退出行为测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

const CHANNEL_ID: &str = "chfixx0000000000000000008d";

/// Runs the public leave command through HTTP and the local persist barrier.
fn run_leave() -> Result<Vec<Value>, String> {
    let mut harness =
        common::Harness::new_with_identity(&[0x8d], "g08d-actor", "g08d-company", "", "", "");
    harness.step(
        common::command_json("im_channel_leave", json!({"channel_id": CHANNEL_ID})),
        1_700_001_600_020,
    );
    let http_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("channel/member/leave") => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing leave HTTP effect".to_string())?;
    if harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. }))
    {
        return Err("leave must not emit before Go authority reply".to_string());
    }

    harness.step(
        Tick::PortReply {
            corr: http_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_600_021,
    );
    let (persist_corr, deletes_viewer_member) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((
                *corr,
                ops.iter().any(|op| {
                    matches!(
                        op,
                        StorageOp::BatchDelete(spec)
                            if spec.table == "channel_member"
                                && spec.scope_col == "channel_id"
                                && spec.key_col == "user_id"
                                && matches!(
                                    spec.key_vals.as_slice(),
                                    [helix_core::effect::SqlValue::Text(user_id)]
                                        if user_id == "g08d-actor"
                                )
                    )
                }),
            )),
            _ => None,
        })
        .ok_or_else(|| "missing leave persistence barrier".to_string())?;
    if !deletes_viewer_member {
        return Err("leave persist must delete the authenticated viewer member row".to_string());
    }
    if harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. }))
    {
        return Err("leave must not emit before PersistOk".to_string());
    }

    harness.step(
        Tick::PortReply {
            corr: persist_corr,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_600_022,
    );
    Ok(harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect())
}

/// G08d keeps leave on the existing channelClosed kind and exposes its reason only after PersistOk.
#[test]
fn g08d_emits_left_terminal_reason_after_persist() -> Result<(), String> {
    let events = run_leave()?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:channel:closed");
    assert_eq!(events[0]["data"]["channelId"], CHANNEL_ID);
    assert_eq!(events[0]["data"]["terminalReason"], "left");
    assert!(events[0]["data"].get("left").is_none());
    Ok(())
}

/// Receiver member changes retain one roster event and never reintroduce per-user leave kinds.
#[test]
fn g08d_member_projection_uses_one_canonical_roster_kind() -> Result<(), String> {
    let effect = helix_im::event::channel_member::updated(json!({
        "channelId": CHANNEL_ID,
        "members": [],
        "leaves": ["g08d-actor"]
    }))
    .map_err(|error| error.to_string())?
    .into_effect();
    let Effect::Emit { event } = effect else {
        return Err("member projection must be an Emit effect".to_string());
    };
    let envelope: Value =
        serde_json::from_slice(event.0.as_ref()).map_err(|error| error.to_string())?;
    assert_eq!(envelope["event"], "im:channel:member-updated");
    assert_eq!(envelope["data"]["leaves"], json!(["g08d-actor"]));
    Ok(())
}
