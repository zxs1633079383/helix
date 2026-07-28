//! G-11a 未缓存消息定位、双向分页与旧 locate response 正式 Gate。

#[path = "support/timeline_formal_gate.rs"]
mod support;

use helix_im::interaction_frame::{TimelineDeltaOperationVm, TimelineInsertPositionVm};
use support::{assert_canonical_gate, delta_operations, rows, TimelineProductionRunner};

/// 发起两个 locate intent 并乱序回放，证明旧 response 只能合并数据、不能激活。
fn produce() -> Vec<(
    &'static str,
    helix_im::interaction_frame::InteractionFrameV2,
)> {
    let channel_id = "chfixx0000000000000000110a";
    let mut runner = TimelineProductionRunner::new(channel_id, "latest");
    let native_attach = runner.attach_latest(Vec::new(), 20);
    let latest = runner.attach_latest(rows(80..100, channel_id), 20);
    let latest_range_id = "latest".to_string();
    let stale_pending = runner.start_locate("server-015", 20, true, true, "navigation-stale");
    let current_pending = runner.start_locate("server-040", 20, true, true, "navigation-current");
    let located = runner.complete_locate(current_pending, rows(30..50, channel_id));
    let located_range_ids = delta_operations(&located)
        .iter()
        .filter_map(|operation| match operation {
            TimelineDeltaOperationVm::InstallSegment { range, .. } => Some(range.range_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(located_range_ids.len(), 1);
    assert_ne!(located_range_ids[0], latest_range_id);
    let located_range_id = located_range_ids[0].clone();
    assert!(delta_operations(&located).iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::ActivateWindow {
            window,
            navigation_token: Some(token),
            ..
        } if token == "navigation-current"
            && window.range_ids == vec![located_range_id.clone()]
    )));
    let stale = runner.complete_locate(stale_pending, rows(0..20, channel_id));
    assert!(delta_operations(&stale)
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::UpsertMessage { .. })));
    let stale_range_ids = delta_operations(&stale)
        .iter()
        .filter_map(|operation| match operation {
            TimelineDeltaOperationVm::InstallSegment { range, .. } => Some(range.range_id.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(stale_range_ids.len(), 1);
    assert_ne!(stale_range_ids[0], latest_range_id);
    assert_ne!(stale_range_ids[0], located_range_id);
    let older = runner.load_older("server-030", rows(20..30, channel_id), 60, true);
    let newer = runner.load_newer("server-049", rows(50..60, channel_id), 60, true);
    assert!(!delta_operations(&older)
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::InstallSegment { .. })));
    assert!(delta_operations(&older).iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            range_id,
            position: TimelineInsertPositionVm::Start,
            ..
        } if range_id == &located_range_id
    )));
    assert!(!delta_operations(&newer)
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::InstallSegment { .. })));
    assert!(delta_operations(&newer).iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            range_id,
            position: TimelineInsertPositionVm::End,
            ..
        } if range_id == &located_range_id
    )));
    for delta in [&older, &newer, &stale] {
        assert!(!delta_operations(delta)
            .iter()
            .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));
    }
    vec![
        ("attach-empty.snapshot.frame-v3.json", native_attach),
        ("latest-loaded.delta.frame-v3.json", latest),
        ("uncached-located.delta.frame-v3.json", located),
        ("located-older.delta.frame-v3.json", older),
        ("located-newer.delta.frame-v3.json", newer),
        ("stale-navigation.delta.frame-v3.json", stale),
    ]
}

/// 校验 G-11a producer、Golden bytes 与 provenance 同源。
#[test]
fn g11a_uncached_locate_matches_canonical_fixtures() {
    assert_canonical_gate(
        "G-11a",
        "g11a_wdio_fixture_contract_test",
        "g11a-wdio",
        "UPDATE_G11A_TIMELINE_FIXTURES",
        &produce(),
    );
}
