//! G-10b loadNewer 下拉分页正式 Gate 与 canonical JSON。

#[path = "support/timeline_formal_gate.rs"]
mod support;

use helix_im::interaction_frame::{
    TimelineDeltaOperationVm, TimelineInsertPositionVm, TypedSurfaceMutation,
};
use support::{assert_canonical_gate, delta_operations, rows, TimelineProductionRunner};

/// 生成可独立 apply 的首屏 Snapshot、定位 Delta、下拉续页、耗尽与晚到消息。
fn produce() -> Vec<(
    &'static str,
    helix_im::interaction_frame::InteractionFrameV2,
)> {
    let channel_id = "chfixx0000000000000000100b";
    let mut runner = TimelineProductionRunner::new(channel_id, "g10b-window");
    let latest = runner.attach_latest(rows(10..30, channel_id), 20);
    assert_eq!(latest.mutations[0].mutation.revision, 1);
    assert_eq!(latest.mutations[0].mutation.base_revision, None);
    assert!(matches!(
        latest.mutations[0].mutation.mutation,
        TypedSurfaceMutation::TimelineSnapshot { .. }
    ));
    let initial = runner.locate(
        "server-019",
        rows(10..30, channel_id),
        20,
        true,
        true,
        "g10b-locate-current",
    );
    assert_eq!(initial.mutations[0].mutation.revision, 2);
    assert_eq!(initial.mutations[0].mutation.base_revision, Some(1));
    assert!(matches!(
        initial.mutations[0].mutation.mutation,
        TypedSurfaceMutation::TimelineDelta { .. }
    ));
    assert!(delta_operations(&initial)
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::InstallSegment { .. })));
    let loaded = runner.load_newer("server-029", rows(30..50, channel_id), 60, true);
    assert!(delta_operations(&loaded).iter().any(|operation| matches!(
        operation,
        TimelineDeltaOperationVm::InsertKeys {
            position: TimelineInsertPositionVm::End,
            ..
        }
    )));
    let exhausted = runner.load_newer("server-049", Vec::new(), 60, false);
    let late = runner.load_newer("server-049", rows(50..51, channel_id), 60, false);
    for delta in [&loaded, &exhausted, &late] {
        assert!(!delta_operations(delta)
            .iter()
            .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));
    }
    vec![
        ("latest.snapshot.frame-v3.json", latest),
        ("initial-located.delta.frame-v3.json", initial),
        ("newer-loaded.delta.frame-v3.json", loaded),
        ("newer-exhausted.delta.frame-v3.json", exhausted),
        ("newer-late.delta.frame-v3.json", late),
    ]
}

/// 校验 G-10b producer、操作合同、Golden bytes 与 provenance 同源。
#[test]
fn g10b_load_newer_formal_gate_matches_canonical_fixtures() {
    assert_canonical_gate(
        "G-10b",
        "g10b_wdio_fixture_contract_test",
        "g10b-wdio",
        "UPDATE_G10B_TIMELINE_FIXTURES",
        &produce(),
    );
}
