//! G-10c 满窗口 append 与头部淘汰正式 Gate。

#[path = "support/timeline_formal_gate.rs"]
mod support;

use helix_im::interaction_frame::{TimelineDeltaOperationVm, TimelineInsertPositionVm};
use support::{assert_canonical_gate, delta_operations, rows, TimelineProductionRunner};

/// 校验 producer 只尾插新 key，头部淘汰仍由 consumer 本地窗口选择器负责。
#[test]
fn g10c_full_window_append_matches_canonical_fixtures() {
    let channel_id = "chfixx0000000000000000100c";
    let mut runner = TimelineProductionRunner::new(channel_id, "latest");
    let initial_rows = rows(0..60, channel_id);
    let snapshot = runner.attach_latest(initial_rows.clone(), 60);
    let appended = runner.send_and_readback_append(initial_rows);
    let operations = delta_operations(&appended);
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
    assert_canonical_gate(
        "G-10c",
        "g10c_wdio_fixture_contract_test",
        "g10c-wdio",
        "UPDATE_G10C_TIMELINE_FIXTURES",
        &[
            ("full-window.snapshot.frame-v3.json", snapshot),
            ("new-message-appended.delta.frame-v3.json", appended),
        ],
    );
}
