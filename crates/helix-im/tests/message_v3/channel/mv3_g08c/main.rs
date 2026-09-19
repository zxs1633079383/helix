use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const CHANNEL_ID: &str = "chfixx0000000000000000008c";
type TestResult<T> = Result<T, String>;

/// 通过既有 channel_close WS handler 和持久化回执收集终态事件。
fn run_persona(user_id: &str) -> TestResult<Vec<Value>> {
    let mut harness =
        common::Harness::new_with_identity(&[0x8c], user_id, "g08c-company", "", "", "");
    let inbound = serde_json::to_vec(&json!({
        "action": "channel_close",
        "data": {"channelId": CHANNEL_ID, "deleteAt": 1_700_001_600_010_i64}
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
        .ok_or_else(|| "missing channel close persistence barrier".to_string())?;
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

/// sender 与 receiver 都只收到既有 channelClosed 终态，且事件发生在 PersistOk 之后。
#[test]
fn g08c_emits_one_closed_projection_after_persist() -> TestResult<()> {
    for user_id in ["g08c-actor", "g08c-observer"] {
        let events = run_persona(user_id)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "im:channel:closed");
        assert_eq!(events[0]["data"]["channelId"], CHANNEL_ID);
        assert_eq!(events[0]["data"]["closed"], true);
    }
    Ok(())
}

/// Rust producer 仍复用单一 event kind，不引入第二条总线或同义事件。
#[test]
fn g08c_reuses_existing_channel_closed_event() -> TestResult<()> {
    let events = run_persona("g08c-actor")?;
    let event = events
        .first()
        .ok_or_else(|| "missing close event".to_string())?;
    assert_eq!(event.as_object().map(|value| value.len()), Some(2));
    assert_eq!(event["event"], "im:channel:closed");
    assert!(event["data"].get("channelId").is_some());
    assert!(event["data"].get("closed").is_some());
    Ok(())
}
