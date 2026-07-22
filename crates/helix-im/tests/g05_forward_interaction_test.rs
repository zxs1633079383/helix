//! G05 focused contract: forward binding, deterministic assembly and per-target partial terminal.

mod common;
use common::*;

use helix_core::Effect;
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[test]
fn forward_capability_produces_an_opaque_live_binding() {
    let channel_id = cid(5);
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest(channel_id.clone()),
            &[json!({
                "id": sid(5), "channelId": channel_id, "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "forward me",
                "sendStatus": "sent", "props": {},
                "actionCapabilities": { "canForward": true }
            })],
        )
        .expect("timeline fixture must project");
    let actions = match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    };
    let forward = actions
        .iter()
        .find(|action| action.kind == "forward")
        .expect("forward binding");
    assert!(
        forward.enabled,
        "TARGET: canForward must be invokable through its opaque actionRef"
    );
}

#[test]
fn per_target_partial_acceptance_never_claims_all_success_or_delivery() {
    let mut harness = Harness::new(&[]);
    harness.step(
        command_json(
            "im_create_posts",
            json!({
                "req_id": "forward-partial-1",
                "posts": [{"id": ""}],
                "channel_ids": [cid(50), cid(51)]
            }),
        ),
        1,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("createPosts HTTP correlation");
    harness.step(
        port_reply_with_body(
            corr,
            enveloped(
                serde_json::to_vec(&json!({
                    "status": "SUCCESS",
                    "data": { "targets": [
                        {"channelId": cid(50), "status": "accepted"},
                        {"channelId": cid(51), "status": "failed", "error": "forbidden"}
                    ]}
                }))
                .unwrap()
                .into(),
            ),
        ),
        2,
    );
    let data = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .filter(|event| event["event"] == "im:posts:batch-result")
        .expect("batch result");
    assert_eq!(data["data"]["batchStatus"], "partial");
    assert_eq!(data["data"]["acceptedCount"], 1);
    assert_eq!(data["data"]["failedCount"], 1);
    assert_eq!(data["data"]["targets"][0]["deliveryStatus"], "pending");
}
