//! Timeline V3 普通更新与满窗口 append 的最小操作合同。

use helix_im::interaction_frame::{
    TimelineDeltaOperationVm, TimelineInsertPositionVm, TypedSurfaceMutation,
};
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

/// 构造稳定 EntityKey 的 render-ready 消息。
fn row(index: usize, status: &str, revoked: bool) -> serde_json::Value {
    json!({
        "id": format!("server-{index}"),
        "temporaryId": format!("temporary-{index:03}"),
        "channelId": "channel-delta",
        "userId": "viewer",
        "createAt": index as i64 + 1,
        "type": "TEXT",
        "message": format!("message {index}"),
        "sendStatus": status,
        "revoke": revoked,
        "props": {},
        "actionCapabilities": {}
    })
}

/// 从 Timeline mutation 中取得有序 Delta operations。
fn operations(
    mutation: helix_im::interaction_frame::SurfaceMutation,
) -> Vec<TimelineDeltaOperationVm> {
    let TypedSurfaceMutation::TimelineDelta { operations } = mutation.mutation else {
        panic!("ordinary update must be timelineDelta");
    };
    operations
}

/// 证明 sent/failed/recalled 等普通终态只 Upsert 同一个稳定 EntityKey。
#[test]
fn ordinary_message_terminal_update_is_one_stable_upsert() {
    for (status, revoked) in [("sent", false), ("failed", false), ("sent", true)] {
        let mut projector = TimelineProjector::default();
        projector
            .snapshot_from_render_ready(
                TimelineWindowRequest::latest_with_window_token("channel-delta", "ordinary-update"),
                &[row(1, "sending", false)],
            )
            .expect("initial snapshot");
        let operations = operations(
            projector
                .patch_from_render_ready(
                    TimelineWindowRequest::latest_with_window_token(
                        "channel-delta",
                        "ordinary-update",
                    ),
                    &[row(1, status, revoked)],
                    None,
                )
                .expect("terminal delta"),
        );
        assert_eq!(operations.len(), 1);
        let TimelineDeltaOperationVm::UpsertMessage { entity } = &operations[0] else {
            panic!("terminal update must be exactly one Upsert");
        };
        assert_eq!(entity.entity_key.temporary_id, "temporary-001");
        assert_eq!(entity.entity_key.create_at, 2);
    }
}

/// 证明新消息只 Upsert 并尾插，离底/普通 append 不隐式 Activate。
#[test]
fn live_append_uses_tail_insert_without_activation() {
    let mut projector = TimelineProjector::default();
    projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest_with_window_token("channel-delta", "append"),
            &[row(1, "sent", false), row(2, "sent", false)],
        )
        .expect("initial snapshot");
    let operations = operations(
        projector
            .patch_from_render_ready(
                TimelineWindowRequest::latest_with_window_token("channel-delta", "append"),
                &[
                    row(1, "sent", false),
                    row(2, "sent", false),
                    row(3, "sent", false),
                ],
                None,
            )
            .expect("append delta"),
    );
    assert!(matches!(
        operations.first(),
        Some(TimelineDeltaOperationVm::UpsertMessage { .. })
    ));
    assert!(operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            position: TimelineInsertPositionVm::End,
            ..
        }
    )));
    assert!(operations
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::SetRangeBounds { .. })));
    assert!(!operations
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));
}

/// 证明满窗口向尾部推进仍不产生 replace/install/delete/trim 或隐式激活。
#[test]
fn full_window_append_is_tail_delta_without_producer_trim() {
    let mut projector = TimelineProjector::default();
    let initial = (0..60)
        .map(|index| row(index, "sent", false))
        .collect::<Vec<_>>();
    projector
        .snapshot_from_render_ready(
            TimelineWindowRequest::latest_with_window_token("channel-delta", "full"),
            &initial,
        )
        .expect("full snapshot");
    let advanced = (1..=60)
        .map(|index| row(index, "sent", false))
        .collect::<Vec<_>>();
    let operations = operations(
        projector
            .patch_from_render_ready(
                TimelineWindowRequest::latest_with_window_token("channel-delta", "full"),
                &advanced,
                None,
            )
            .expect("full append delta"),
    );
    assert!(operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::UpsertMessage { entity }
            if entity.entity_key.temporary_id == "temporary-060"
    )));
    assert!(operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            position: TimelineInsertPositionVm::End,
            keys,
            ..
        } if keys.len() == 1
    )));
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InstallSegment { .. }
            | TimelineDeltaOperationVm::ActivateWindow { .. }
    )));
}
