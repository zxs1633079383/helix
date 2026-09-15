//! Regression: receipt events occupy the canonical stream even while storage is pending.
mod common;
use common::*;
use helix_core::{
    tick::{InboundBytes, PortOutcome},
    Tick,
};

/// Stream fixtures intentionally include channel-level type6 without a message id.
fn stream(seq: u64, redacted: bool) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&serde_json::json!({
            "action": "channel_stream_event", "data": {
                "channel_id": cid(1), "stream_seq": seq, "event_id": format!("event-{seq}"),
                "event_type": if seq == 1 {6} else {1}, "redacted": redacted
            }
        }))
        .unwrap()
        .into(),
    ))
}

/// Extract the single serialized durable write without fabricating a correlation.
fn atomic(effects: &[EffectKind]) -> u64 {
    let ids: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            EffectKind::PersistAtomic(c) => Some(*c),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 1, "{effects:?}");
    ids[0]
}

/// An ordered burst waits locally and drains one acknowledged event at a time.
#[test]
fn contiguous_burst_waits_without_gap_and_reserves_every_next_slot() {
    for redacted in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
        let mut corr = atomic(&h.step(stream(1, redacted), 1000));
        for seq in 2..=32 {
            let effects = h.step(stream(seq, redacted), 1000 + seq);
            assert!(
                effects.is_empty(),
                "ordered event {seq} is not a gap: {effects:?}"
            );
        }
        for seq in 1..=32 {
            assert_eq!(h.module.cursor_for(channel_id(1)), Some(seq - 1));
            let effects = h.step(port_reply_ok(corr), 2000 + seq);
            assert_eq!(h.module.cursor_for(channel_id(1)), Some(seq));
            if seq < 32 {
                corr = atomic(&effects);
                assert!(
                    h.step(stream(seq + 1, redacted), 3000 + seq).is_empty(),
                    "pending duplicate must not write again"
                );
            } else {
                assert!(!effects.iter().any(|e| matches!(
                    e,
                    EffectKind::PersistAtomic(_)
                        | EffectKind::Http(_)
                        | EffectKind::ScheduleTimer(_)
                )));
            }
        }
    }
}

/// A genuinely absent sequence still requests server recovery immediately.
#[test]
fn missing_sequence_still_recovers() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    h.step(stream(1, true), 1000);
    let effects = h.step(stream(3, true), 1001);
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Http(_))),
        "{effects:?}"
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
}

/// Failure must not commit the cursor and must retain a recovery timer.
#[test]
fn persistence_failure_retains_cursor_and_recovers() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let corr = atomic(&h.step(stream(1, true), 1000));
    h.step(stream(2, true), 1001);
    let effects = h.step(
        Tick::PortReply {
            corr: helix_core::Correlation::from_raw(corr),
            outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
        },
        1002,
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "{effects:?}"
    );
    let timer = effects
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(t) => Some(*t),
            _ => None,
        })
        .unwrap();
    let recovery = h.step(timer_tick(timer), 2003);
    assert!(recovery.iter().any(|e| matches!(e, EffectKind::Http(_))));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
}

/// Filling an observed hole cancels its timer while the contiguous writes are pending.
#[test]
fn filled_hole_cancels_timer_and_drains_in_order() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let first = atomic(&h.step(stream(1, true), 1000));
    let gap = h.step(stream(3, true), 1001);
    let timer = gap
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(t) => Some(*t),
            _ => None,
        })
        .unwrap();
    let filled = h.step(stream(2, true), 1002);
    assert!(
        filled.contains(&EffectKind::CancelTimer(timer)),
        "{filled:?}"
    );
    assert!(h.step(timer_tick(timer), 2002).is_empty());
    let second = atomic(&h.step(port_reply_ok(first), 2003));
    let third = atomic(&h.step(port_reply_ok(second), 2004));
    h.step(port_reply_ok(third), 2005);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(3));
}

/// Legacy author receipt can arrive first; only canonical acknowledgement advances cursor.
#[test]
fn author_receipt_then_canonical_then_message_is_contiguous() {
    let mut h = Harness::new_with_identity(&[1], "author", "tenant", "", "", "");
    let receipt = Tick::Inbound(InboundBytes(
        serde_json::to_vec(&serde_json::json!({
            "action":"post_read", "data": {
                "channelId":cid(1), "postId":sid(1), "eventSeq":1,
                "authorUserId":"author", "readerUserId":"reader", "snapshotId":"snapshot",
                "memberIds":["reader"], "readMap":"1", "updateAt":10
            }
        }))
        .unwrap()
        .into(),
    ));
    let effects = h.step(receipt, 1000);
    let read_corr = effects
        .iter()
        .find_map(|e| match e {
            EffectKind::Persist(c) => Some(*c),
            _ => None,
        })
        .unwrap();
    h.step(port_reply_ok(read_corr), 1001);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
    let first = atomic(&h.step(stream(1, false), 1002));
    assert!(h.step(stream(2, false), 1003).is_empty());
    let second = atomic(&h.step(port_reply_ok(first), 1004));
    h.step(port_reply_ok(second), 1005);
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(2));
}
