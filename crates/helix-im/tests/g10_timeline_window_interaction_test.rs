//! G10 focused contract: a bounded window is a complete replacement VM.

use helix_im::interaction_frame::{
    SurfaceLoadState, TimelineAnchorMode, TimelineAnchorVm, TypedSurfaceMutation, WindowPageVm,
};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[path = "support/g10_scenario.rs"]
mod scenario;

/// 构造 projector-only 的 render-ready timeline row。
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

/// 证明窗口更新只允许完整 replaceView，并保持调用方的 opaque token。
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

/// 证明生产 cache persist barrier 后首帧为 snapshot，恢复帧单调变为 patch。
#[test]
fn production_recovery_advances_revision_without_changing_scope() {
    let trace = scenario::G10ScenarioRunner::run_success_and_recovery();
    let ready = &trace.ready.mutations[0].mutation;
    let recovered = &trace.recovered.mutations[0].mutation;
    assert_eq!(ready.scope, recovered.scope);
    assert_eq!(recovered.base_revision, Some(ready.revision));
    assert!(recovered.revision > ready.revision);
}

/// 证明迟到与重复 PortReply 均不能追加 revision 或 terminal Frame。
#[test]
fn production_query_generation_drops_late_and_duplicate_replies() {
    let trace = scenario::G10ScenarioRunner::run_latest_query_wins();
    assert_eq!(trace.late_effect_count, 0);
    assert_eq!(trace.duplicate_effect_count, 0);
    scenario::assert_timeline_contract(
        &trace.current,
        SurfaceLoadState::Ready,
        Some("g10-current"),
        true,
    );
}
