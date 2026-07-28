//! G10 focused contract: a bounded window advances through Timeline V3 Range operations.

use helix_im::interaction_frame::{
    SurfaceLoadState, TimelineAnchorMode, TimelineAnchorVm, TimelineDeltaOperationVm,
    TimelineInsertPositionVm, TypedSurfaceMutation, WindowPageVm,
};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

#[path = "support/g10a_scenario.rs"]
#[allow(dead_code)]
mod older_scenario;
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

/// 证明上拉只追加实体/Range keys 与边界，不再覆盖整个窗口。
#[test]
fn older_window_patch_inserts_range_keys_and_keeps_token() {
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
    let TypedSurfaceMutation::TimelineDelta { operations } = mutation.mutation else {
        panic!("window update must be a Timeline V3 delta")
    };
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, TimelineDeltaOperationVm::UpsertMessage { .. }))
            .count(),
        3
    );
    assert!(operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            range_id,
            position: TimelineInsertPositionVm::Start,
            keys,
        } if range_id == "window-10" && keys.len() == 3
    )));
    assert!(operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::SetRangeBounds {
            range_id,
            has_older: true,
            has_newer: true,
            ..
        } if range_id == "window-10"
    )));
    assert!(!matches!(
        operations.as_slice(),
        [TimelineDeltaOperationVm::InstallSegment { .. }]
    ));
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

/// Proves G10a advances the same scope only after message persistence and durable readback.
#[test]
fn production_load_older_uses_the_existing_timeline_revision_chain() {
    let trace = older_scenario::G10AScenarioRunner::run_success();
    assert!(trace.persisted_before_frame);
    assert_eq!(
        trace.initial.mutations[0].mutation.scope,
        trace.loaded.mutations[0].mutation.scope
    );
    assert_eq!(
        trace.loaded.mutations[0].mutation.base_revision,
        Some(trace.initial.mutations[0].mutation.revision)
    );
}

/// Proves G10a duplicate PortReplies cannot append another terminal timeline revision.
#[test]
fn production_load_older_drops_consumed_correlations() {
    let trace = older_scenario::G10AScenarioRunner::run_duplicate_replies();
    assert_eq!(trace.duplicate_persist_effects, 0);
    assert_eq!(trace.duplicate_readback_effects, 0);
}
