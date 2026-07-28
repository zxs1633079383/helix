//! Timeline V3 latest Snapshot 的 pageSize、lookahead 与稳定排序合同。

use helix_im::interaction_frame::TypedSurfaceMutation;
use helix_im::projector::timeline::{TimelineProjector, TimelineWindowRequest};
use serde_json::json;

/// 构造包含稳定复合身份的 render-ready 消息。
fn row(index: usize, create_at: i64) -> serde_json::Value {
    json!({
        "id": format!("server-{index}"),
        "temporaryId": format!("temporary-{index:03}"),
        "channelId": "channel-latest",
        "userId": "viewer",
        "createAt": create_at,
        "type": "TEXT",
        "message": format!("message {index}"),
        "sendStatus": "sent",
        "props": {},
        "actionCapabilities": {}
    })
}

/// 证明最新页只携带请求数量，额外 lookahead 只改变 hasOlder。
#[test]
fn latest_snapshot_uses_requested_page_size_and_excludes_lookahead() {
    let rows = (0..21)
        .map(|index| row(index, index as i64 + 1))
        .collect::<Vec<_>>();
    let mut request =
        TimelineWindowRequest::latest_with_window_token("channel-latest", "latest-20");
    request.page_size = 20;
    let mutation = TimelineProjector::default()
        .snapshot_from_render_ready(request, &rows)
        .expect("latest snapshot projects");
    let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
        panic!("latest attach must produce snapshot");
    };
    assert_eq!(view.entities.len(), 20);
    assert_eq!(view.entities[0].entity_key.temporary_id, "temporary-001");
    assert_eq!(
        view.entities.last().unwrap().entity_key.temporary_id,
        "temporary-020"
    );
    assert!(view.ranges[0].has_older);
    assert!(!view.ranges[0].has_newer);
}

/// 证明同毫秒消息仍以 temporaryId 保持稳定、唯一的协议顺序。
#[test]
fn latest_snapshot_preserves_stable_composite_identity_for_equal_timestamps() {
    let rows = vec![row(1, 10), row(2, 10), row(3, 10)];
    let mut request =
        TimelineWindowRequest::latest_with_window_token("channel-latest", "latest-equal");
    request.page_size = 20;
    let mutation = TimelineProjector::default()
        .snapshot_from_render_ready(request, &rows)
        .expect("equal timestamps remain valid");
    let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
        panic!("latest attach must produce snapshot");
    };
    let keys = view.ranges[0]
        .ordered_keys
        .iter()
        .map(|key| (key.create_at, key.temporary_id.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        vec![
            (10, "temporary-001"),
            (10, "temporary-002"),
            (10, "temporary-003")
        ]
    );
}
