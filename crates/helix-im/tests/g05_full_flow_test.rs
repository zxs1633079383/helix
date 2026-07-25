//! G05 转发/合并转发的 production `ImModule::handle` 完整流程合同。

#[path = "support/g05_scenario.rs"]
mod scenario;

use scenario::{
    assert_source_forwardable_contract, assert_target_dialog_contract,
    assert_target_dialog_contract_for, assert_target_forwarded_contract, first_dialog_row,
    surface_names, G05ScenarioRunner, TARGET_CHANNEL_ID,
};

#[test]
/// 证明 canonical G05 从 fresh target group 与 Helix 签发的 source action 开始。
fn prerequisite_creates_fresh_target_and_forwardable_source() {
    let trace = G05ScenarioRunner::run_happy_ws_first();
    assert_eq!(
        first_dialog_row(&trace.group_created)
            .expect("fresh group dialog row")
            .channel_id,
        TARGET_CHANNEL_ID
    );
    assert_source_forwardable_contract(&trace.source_forwardable);
}

#[test]
/// 证明可信 WS authority 经过 message/dialog 双持久屏障后产出完整 target Frames。
fn happy_path_projects_target_timeline_and_terminal_dialog() {
    let trace = G05ScenarioRunner::run_happy_ws_first();
    assert_target_forwarded_contract(&trace.target_forwarded);
    assert_target_dialog_contract(&trace.target_dialog);
    assert_eq!(trace.http_accepted_frame_count, 0);
}

#[test]
/// 证明 per-target 业务拒绝不伪造 target message 或 terminal Frame。
fn business_failure_does_not_claim_delivery() {
    let trace = G05ScenarioRunner::run_business_failure();
    assert!(surface_names(&trace.group_created).contains("message.left-dialog-list"));
    assert_source_forwardable_contract(&trace.source_forwardable);
    assert_eq!(trace.terminal_frame_count, 0);
    assert_eq!(trace.target_message_write_count, 0);
}

#[test]
/// 证明 HTTP 基础设施失败保留既有 source timeline 且无 target 写入。
fn infrastructure_failure_keeps_source_state() {
    let trace = G05ScenarioRunner::run_infrastructure_failure();
    assert!(surface_names(&trace.group_created).contains("message.left-dialog-list"));
    assert_source_forwardable_contract(&trace.source_forwardable);
    assert_eq!(trace.terminal_frame_count, 0);
    assert_eq!(trace.target_message_write_count, 0);
}

#[test]
/// 证明 WS 先完成后迟到 HTTP 与重复 inbound 均不重复发布 terminal Frame。
fn ws_first_race_and_duplicate_inbound_are_idempotent() {
    let trace = G05ScenarioRunner::run_happy_ws_first();
    assert_target_dialog_contract(&trace.target_dialog);
    assert_eq!(trace.late_http_frame_count, 0);
}

#[test]
/// 证明 dialog PersistErr 不消费 authority，后续可信 post 可重放收敛。
fn persistence_failure_is_replayable() {
    let trace = G05ScenarioRunner::run_persist_failure_replay();
    assert!(surface_names(&trace.group_created).contains("message.left-dialog-list"));
    assert_source_forwardable_contract(&trace.source_forwardable);
    assert_target_forwarded_contract(&trace.target_forwarded);
    assert_target_dialog_contract_for(&trace.target_dialog_after_replay, 2);
}
