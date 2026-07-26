//! G16b member leave 广播的 Helix sans-IO 完整流程合同。

#[path = "support/g15a_scenario.rs"]
mod prerequisite;
#[path = "support/g16b_scenario.rs"]
mod scenario;

use scenario::{assert_roster_contract, G16bScenarioRunner, LEAVING_USER_ID, REMAINING_USER_ID};

#[test]
/// 证明每次 G16b canonical 验收先建立 fresh group authority。
fn prerequisite_creates_fresh_group_before_member_leave_broadcast() {
    let created = prerequisite::G15aScenarioRunner::run_http_created();
    prerequisite::assert_created_contract(&created.created, Some(prerequisite::REQUEST_ID));
}

#[test]
/// 证明 WS leave 仅在 PersistOk 与 durable readback 后发布完整 roster patch。
fn member_leave_broadcast_is_durable_and_field_complete() {
    let trace = G16bScenarioRunner::run_leave();
    assert_roster_contract(&trace.initial, &[LEAVING_USER_ID, REMAINING_USER_ID], false);
    assert_roster_contract(&trace.terminal, &[REMAINING_USER_ID], true);
    assert!(trace.persist_corr.raw() > 0);
    assert!(trace.readback_corr.raw() > trace.persist_corr.raw());
}

#[test]
/// 证明持久失败不出帧且同一 authority 可在失败后重放。
fn persist_failure_is_fail_closed_and_replayable() {
    let (frame_count, replay_count) = G16bScenarioRunner::run_persist_failure();
    assert_eq!(frame_count, 0);
    assert_eq!(replay_count, 1);
}

#[test]
/// 证明同一 event_seq 已提交后重复 inbound 不产生写或 Frame。
fn duplicate_member_leave_authority_is_single_use() {
    assert_eq!(G16bScenarioRunner::run_leave().duplicate_effect_count, 0);
}

#[test]
/// 证明未 attach 时不要求 Frame，后续 attach 可从 durable roster 恢复。
fn recovery_rebuilds_roster_from_durable_member_rows() {
    assert_roster_contract(
        &G16bScenarioRunner::run_recovery(),
        &[REMAINING_USER_ID],
        false,
    );
}
