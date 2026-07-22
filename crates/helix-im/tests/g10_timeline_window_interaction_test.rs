//! G10 focused contract: a bounded window is a complete replacement VM.

use helix_im::interaction_frame::{
    SurfaceLoadState, TimelineAnchorMode, TimelineAnchorVm, TypedSurfaceMutation, WindowPageVm,
};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

fn row(index: usize) -> serde_json::Value {
    json!({
        "id": format!("message-{index}"),
        "channelId": "channel-10",
        "userId": "444",
        "createdAt": index,
        "type": "TEXT",
        "message": format!("message {index}"),
        "sendStatus": "sent",
        "props": {},
        "actionCapabilities": {}
    })
}

#[test]
fn older_window_patch_replaces_the_complete_bounded_view_and_keeps_token() {
    let mut projector = TimelineProjector::default();
    let mut request = TimelineWindowRequest::latest_with_window_token("channel-10", "window-10");
    projector
        .snapshot_from_render_ready(request.clone(), &[row(21), row(22)])
        .unwrap();
    request.anchor = TimelineAnchorVm {
        message_id: Some("message-20".into()),
        mode: TimelineAnchorMode::Latest,
    };
    request.page = WindowPageVm {
        window_token: "window-10".into(),
        has_older: true,
        has_newer: true,
        has_more: true,
    };
    request.state = SurfaceLoadState::Ready;
    let mutation = projector
        .patch_from_render_ready(
            request,
            &[row(18), row(19), row(20), row(21), row(22)],
            None,
        )
        .unwrap();
    let TypedSurfaceMutation::TimelinePatch { patch } = mutation.mutation else {
        panic!("window update must be a complete replaceView patch")
    };
    assert_eq!(patch.view.page.window_token, "window-10");
    assert_eq!(patch.view.items.len(), 5);
    assert!(patch.view.page.has_older && patch.view.page.has_newer);
}
