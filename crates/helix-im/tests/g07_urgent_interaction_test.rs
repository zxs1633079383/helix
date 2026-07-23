//! G07 focused contract: three independent capabilities and strict canonical urgent wire.

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[test]
fn urgent_post_confirm_and_cancel_are_independent_live_bindings() {
    let mut projector = TimelineProjector::default();
    let mutation = projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest("channel-urgent"),
            &[json!({
                "id": "post-urgent", "channelId": "channel-urgent", "userId": "444",
                "createdAt": 1, "type": "TEXT", "message": "urgent",
                "sendStatus": "sent", "props": {},
                "actionCapabilities": {
                    "canUrgent": true,
                    "canUrgentConfirm": true,
                    "canUrgentCancel": true
                }
            })],
        )
        .expect("timeline fixture must project");
    let actions = match mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => view.items[0].actions.clone(),
        other => panic!("expected timeline snapshot, got {other:?}"),
    };
    for kind in ["urgent", "urgentConfirm", "urgentCancel"] {
        assert!(
            actions
                .iter()
                .find(|action| action.kind == kind)
                .unwrap_or_else(|| panic!("missing {kind}"))
                .enabled
        );
    }
}

#[test]
fn urgent_wire_is_strict_and_never_accepts_renderer_owned_authority_fields() {
    let effects = handle_outbound(
        "im_urgent_post",
        &serde_json::to_vec(&json!({
            "post_id": "p1", "target_ids": ["678"], "message": "please confirm", "req_id": "urgent-1"
        })).unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(1),
    ).expect("canonical urgent command");
    let body = match &effects[0] {
        Effect::Http { req, .. } => {
            serde_json::from_slice::<serde_json::Value>(req.body.as_ref().expect("body")).unwrap()
        }
        other => panic!("expected HTTP, got {other:?}"),
    };
    assert_eq!(
        body,
        json!({ "postId": "p1", "targetIds": ["678"], "message": "please confirm" })
    );

    for forbidden in ["user_id", "actor_id"] {
        let mut payload = json!({ "post_id": "p1", "target_ids": ["678"] });
        payload[forbidden] = json!("spoofed");
        assert!(
            handle_outbound(
                "im_urgent_post",
                &serde_json::to_vec(&payload).unwrap(),
                "http://test/api/cses",
                "http://test/api/cses",
                None,
                Correlation::from_raw(2),
            )
            .is_err(),
            "{forbidden} must be rejected"
        );
    }

    let compatibility = handle_outbound(
        "im_urgent_confirm",
        &serde_json::to_vec(&json!({ "post_id": "p1", "channel_id": "legacy-channel" })).unwrap(),
        "http://test/api/cses",
        "http://test/api/cses",
        None,
        Correlation::from_raw(3),
    )
    .expect("legacy public command may carry channel_id for compatibility");
    let body = match &compatibility[0] {
        Effect::Http { req, .. } => {
            serde_json::from_slice::<serde_json::Value>(req.body.as_ref().expect("body")).unwrap()
        }
        other => panic!("expected HTTP, got {other:?}"),
    };
    assert_eq!(
        body,
        json!({ "postId": "p1" }),
        "channel authority remains server-owned"
    );
}
