//! G04a 离线撤回重放的 Helix sans-IO 完整回放合同。

#[path = "support/g04a_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_offline_recalled_contract, assert_revocable_contract,
    g04a_scenario_spec, G04AScenarioRunner,
};

#[test]
/// 证明 G04a 每个 canonical case 都从权威双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_offline_revoke() {
    let trace = G04AScenarioRunner::run_offline_revoke();
    assert_group_created_contract_for(&trace.group_created, &g04a_scenario_spec());
}

#[test]
/// 证明 explicit sync 忽略平台 eventSeq，并在 PersistOk 后投影 recalled。
fn happy_path_uses_helix_cursor_and_projects_recalled_after_persist() {
    let trace = G04AScenarioRunner::run_offline_revoke();
    assert_eq!(
        trace.submitted_from_seq, 0,
        "fresh Helix channel cursor starts at zero"
    );
    assert_revocable_contract(&trace.revocable);
    assert_offline_recalled_contract(&trace.recalled);
}

#[test]
/// 证明 authority no_change 不产生业务写或伪造 Frame。
fn no_change_is_a_business_noop() {
    let trace = G04AScenarioRunner::run_no_change();
    assert_revocable_contract(&trace.revocable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 sync/notify 基础设施错误保留既有可撤回消息。
fn infrastructure_failure_keeps_existing_message_visible() {
    let trace = G04AScenarioRunner::run_http_failure();
    assert_revocable_contract(&trace.revocable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 PersistAtomic 失败不部分发布，新 sync 可重拉同一 authority fact。
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G04AScenarioRunner::run_persist_failure_replay();
    assert_revocable_contract(&trace.revocable);
    assert_offline_recalled_contract(&trace.recalled_after_replay);
}

#[test]
/// 证明已提交 cursor 后重复 sync 与迟到 WS 不重复产生 mutation。
fn duplicate_and_late_authority_are_idempotent() {
    let trace = G04AScenarioRunner::run_offline_revoke();
    assert_offline_recalled_contract(&trace.recalled);
}

#[test]
/// 证明 cold-start 只凭 durable revoked row 恢复 recalled snapshot。
fn recovery_projects_recalled_without_platform_reducer() {
    let trace = G04AScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g04a_scenario_spec());
    assert_offline_recalled_contract(&trace.recovered);
}
