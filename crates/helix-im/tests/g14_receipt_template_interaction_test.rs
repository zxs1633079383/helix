use std::collections::BTreeMap;

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::interaction_frame::{
    ChannelRole, ParticipantReceiptRowVm, ReceiptKind, ReceiptStatus, SurfaceDataVm,
    SurfaceLoadState, UserRefVm,
};
use helix_im::projector::participant_receipts::{
    ParticipantReceiptsData, ParticipantReceiptsProjector, ParticipantReceiptsRequest,
};
use serde_json::{json, Value};

fn one_http(name: &str, payload: Value) -> (String, Value) {
    let effects = handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        Some("conn-g14"),
        Correlation::from_raw(14),
    )
    .expect("canonical receipt command");
    match effects.as_slice() {
        [Effect::Http { req, .. }] => (
            req.url.clone(),
            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
        ),
        other => panic!("expected one HTTP effect, got {other:?}"),
    }
}

fn completed_data() -> ParticipantReceiptsData {
    ParticipantReceiptsData {
        summary: SurfaceDataVm::Record(BTreeMap::new()),
        completed: vec![ParticipantReceiptRowVm {
            user: UserRefVm {
                user_id: "u-1".into(),
            },
            member_role: ChannelRole::Member,
            status: ReceiptStatus::Completed,
            completed_at: Some(1),
        }],
        pending: Vec::new(),
        state: SurfaceLoadState::Ready,
    }
}

#[test]
fn template_received_wire_is_not_a_confirmation_command() {
    let (url, body) = one_http("im_template_received", json!({"post_id":"message-14"}));
    assert_eq!(url, "http://x/api/cses/post/templateReceived");
    assert_eq!(body, json!({"postId":"message-14"}));
}

#[test]
fn template_confirmation_ready_data_is_fail_closed_until_go_has_authority() {
    let mut projector = ParticipantReceiptsProjector::default();
    let request = ParticipantReceiptsRequest::new(
        "channel-14",
        "message-14",
        ReceiptKind::TemplateConfirmation,
        "cursor-14",
    );
    let error = projector.snapshot(request, completed_data()).unwrap_err();
    assert!(error.to_string().contains("authority unavailable"));
}

#[test]
fn template_confirmation_can_only_render_explicit_forbidden_state() {
    let mut projector = ParticipantReceiptsProjector::default();
    let request = ParticipantReceiptsRequest::new(
        "channel-14",
        "message-14",
        ReceiptKind::TemplateConfirmation,
        "cursor-14",
    );
    let mutation = projector.snapshot_forbidden(request).unwrap();
    assert_eq!(mutation.causation_id, None);
}
