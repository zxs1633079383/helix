//! G-10d 离底 anchor 固定与新消息提示正式 Gate。

#[path = "support/timeline_formal_gate.rs"]
mod support;

use helix_im::interaction_frame::{TimelineDeltaOperationVm, TypedSurfaceMutation};
use support::{assert_canonical_gate, delta_operations, rows, TimelineProductionRunner};

/// 校验 inventory 从 revision-1 Snapshot 起步，append Delta 不隐式激活窗口。
#[test]
fn g10d_anchored_append_matches_canonical_fixtures() {
    let channel_id = "chfixx0000000000000000100d";
    let mut runner = TimelineProductionRunner::new(channel_id, "latest");
    let initial_rows = rows(0..20, channel_id);
    let snapshot = runner.attach_latest(initial_rows.clone(), 20);
    assert_eq!(snapshot.mutations[0].mutation.revision, 1);
    assert_eq!(snapshot.mutations[0].mutation.base_revision, None);
    assert!(matches!(
        snapshot.mutations[0].mutation.mutation,
        TypedSurfaceMutation::TimelineSnapshot { .. }
    ));
    let appended = runner.send_and_readback_append(initial_rows);
    assert_eq!(appended.mutations[0].mutation.revision, 2);
    assert_eq!(appended.mutations[0].mutation.base_revision, Some(1));
    assert!(!delta_operations(&appended)
        .iter()
        .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));
    assert_canonical_gate(
        "G-10d",
        "g10d_wdio_fixture_contract_test",
        "g10d-wdio",
        "UPDATE_G10D_TIMELINE_FIXTURES",
        &[
            ("anchored.snapshot.frame-v3.json", snapshot),
            ("anchored-new-message.delta.frame-v3.json", appended),
        ],
    );
}
