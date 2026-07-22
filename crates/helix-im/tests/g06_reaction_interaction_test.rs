//! G06 focused contract: reaction intent is opaque and the Go wire stays postId/emoji only.

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[test]
fn reaction_capability_produces_an_opaque_live_binding() {
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest("channel-reaction"),
            &[json!({
                "id": "post-reaction", "channelId": "channel-reaction", "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "react",
                "sendStatus": "sent", "props": {},
                "actionCapabilities": { "canReact": true }
            })],
        )
        .expect("timeline fixture must project");
    let actions = match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    };
    assert!(
        actions
            .iter()
            .find(|action| action.kind == "react")
            .expect("react binding")
            .enabled
    );
}

#[test]
fn quick_reply_wire_rejects_desired_selected_and_contains_only_post_id_and_emoji() {
    let effects = handle_outbound(
        "im_quick_reply",
        &serde_json::to_vec(&json!({ "post_id": "p1", "emoji": "👍", "req_id": "reaction-1" }))
            .unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(1),
    )
    .expect("canonical reaction command");
    let body = match &effects[0] {
        Effect::Http { req, .. } => {
            serde_json::from_slice::<serde_json::Value>(req.body.as_ref().expect("body")).unwrap()
        }
        other => panic!("expected HTTP, got {other:?}"),
    };
    assert_eq!(body, json!({ "postId": "p1", "emoji": "👍" }));

    let invalid = handle_outbound(
        "im_quick_reply",
        &serde_json::to_vec(&json!({ "post_id": "p1", "emoji": "👍", "desired_selected": true }))
            .unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(2),
    );
    assert!(
        invalid.is_err(),
        "desiredSelected is local intent and must not cross the Go wire"
    );
}
