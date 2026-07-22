//! G04/G04a focused contract: revoke capability and offline authority stay behind persistence.

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

fn timeline_actions(
    capabilities: serde_json::Value,
) -> Vec<helix_im::interaction_frame::ActionBinding> {
    let channel_id = cid(4);
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest(channel_id.clone()),
            &[json!({
                "id": sid(4), "channelId": channel_id, "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "revoke me",
                "sendStatus": "sent", "props": {}, "actionCapabilities": capabilities
            })],
        )
        .expect("timeline fixture must project");
    match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    }
}

#[test]
fn revoke_capability_is_live_only_when_authority_grants_it() {
    let actions = timeline_actions(json!({ "canRevoke": true }));
    let revoke = actions
        .iter()
        .find(|action| action.kind == "revoke")
        .expect("revoke binding");
    assert!(
        revoke.enabled,
        "TARGET: trusted canRevoke must produce a live opaque binding"
    );
    assert!(revoke.disabled_reason.is_none());
}

#[test]
fn offline_revoke_fact_waits_for_one_atomic_persist_before_business_projection() {
    let channel_id = cid(4);
    let message_id = sid(4);
    let mut harness = Harness::new_with_identity(&[4], "678", "team-1", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({
                "action": "posts_update",
                "data": {
                    "event_seq": 1,
                    "channelId": channel_id,
                    "posts": [{
                        "id": message_id,
                        "channelId": cid(4),
                        "userId": "444",
                        "revoke": true,
                        "message": "recalled"
                    }]
                }
            }))
            .unwrap()
            .into(),
        )),
        10,
    );

    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })),
        "G04a offline replay must never create an online revoke HTTP request"
    );
    assert_eq!(
        harness
            .sink
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistAtomic { .. }))
            .count(),
        1,
        "message revoke, viewer indexes and eventSeq must share one atomic write-set"
    );
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "business projection must wait for the matching PersistOk"
    );
}
