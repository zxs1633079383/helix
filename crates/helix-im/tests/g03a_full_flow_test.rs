//! G03a 普通消息失败重试的 Helix sans-IO 完整回放合同。

#[path = "support/g03a_scenario.rs"]
mod scenario;

use scenario::{
    assert_dialog_contract_for, assert_group_created_contract_for, assert_retry_timeline_contract,
    g03a_scenario_spec, G03aScenarioRunner, Terminal, SERVER_ID,
};

#[test]
/// 证明 G03a 每个 case 都从权威双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_retry() {
    let trace = G03aScenarioRunner::run_http(Terminal::Sent);
    assert_group_created_contract_for(&trace.group_created, &g03a_scenario_spec());
}

#[test]
/// 证明失败行以相同 temporaryId 重试，并经 Persist barrier 收敛到 sent。
fn happy_path_reuses_failed_identity_and_projects_retry_sent() {
    let trace = G03aScenarioRunner::run_http(Terminal::Sent);
    assert_retry_timeline_contract(&trace.initial_sending, "sending", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.initial_failed, "failed", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.retry_sending, "sending", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.retry_terminal, "sent", SERVER_ID);
}

#[test]
/// 证明 retry 的远端业务拒绝不会误标 sent，并由 timeout 回到 failed。
fn business_failure_after_retry_returns_to_retryable_failed() {
    let trace = G03aScenarioRunner::run_http(Terminal::BusinessFailed);
    assert_retry_timeline_contract(&trace.retry_sending, "sending", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.retry_terminal, "failed", &trace.temporary_id);
}

#[test]
/// 证明 retry 的 HTTP 基础设施错误经终态持久屏障回到 failed。
fn infrastructure_failure_after_retry_returns_to_failed() {
    let trace = G03aScenarioRunner::run_http(Terminal::Failed);
    assert_retry_timeline_contract(&trace.retry_terminal, "failed", &trace.temporary_id);
}

#[test]
/// 证明 retry 的 WS authority 可先行结算且 timeline/dialog correlation 独立。
fn retry_ws_first_race_is_idempotent_and_updates_dialog() {
    let trace = G03aScenarioRunner::run_ws_echo_with_dialog();
    assert_retry_timeline_contract(&trace.initial_failed, "failed", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.retry_sending, "sending", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.timeline_sent, "sent", SERVER_ID);
    assert_dialog_contract_for(&trace.dialog, &g03a_scenario_spec());
    assert_ne!(trace.timeline_corr, trace.dialog_corr);
}

#[test]
/// 证明冷启动仅凭 durable failed row 可恢复相同 identity 并完成 retry。
fn recovery_rehydrates_failed_row_before_retry() {
    let trace = G03aScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g03a_scenario_spec());
    assert_retry_timeline_contract(&trace.retry_sending, "sending", &trace.temporary_id);
    assert_retry_timeline_contract(&trace.retry_sent, "sent", SERVER_ID);
}
