#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

#[path = "../../../common.rs"]
mod common;

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

const GATE_ID: &str = "MV3-G09c";

/// 锁定 G09c 的完整只读参考包，供后续昵称测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 喂入真实昵称 WS action，并返回持久化 correlation 与回放 harness。
fn nickname_harness() -> (common::Harness, helix_core::Correlation) {
    let mut harness =
        common::Harness::new_with_identity(&[0x9c], "g09c-viewer", "g09c-company", "", "", "");
    let inbound = serde_json::to_vec(&json!({
        "action": "update_channel_member_nickName",
        "data": {
            "channelId": "chfixx0000000000000000009c",
            "userId": "g09c-member",
            "nickName": "群内昵称"
        }
    }))
    .expect("nickname frame must encode");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        1_700_009_000_000,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("nickname update must use a persistence barrier");
    (harness, corr)
}

/// 昵称事件只能在 matching PersistOk 后以 canonical 名称和三字段 payload 释放一次。
#[test]
fn nickname_event_waits_for_persist_ok() {
    let (mut harness, corr) = nickname_harness();
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
        1_700_009_000_001,
    );
    let events = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:channel:member-nickname");
    assert_eq!(events[0]["data"]["channelId"], "chfixx0000000000000000009c");
    assert_eq!(events[0]["data"]["userId"], "g09c-member");
    assert_eq!(events[0]["data"]["nickName"], "群内昵称");
}

/// PersistErr 消费昵称写上下文但不发布成功事件，避免 DOM 领先于 durable storage。
#[test]
fn nickname_persist_error_fails_closed() {
    let (mut harness, corr) = nickname_harness();
    harness.step(
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(PortError::Storage(1)),
        },
        1_700_009_000_001,
    );
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
}
