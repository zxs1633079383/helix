//! G04 在线撤回的 Helix sans-IO 完整回放合同。

#[path = "support/g04_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_recalled_contract, assert_revocable_contract,
    g04_scenario_spec, G04ScenarioRunner,
};

#[test]
/// 证明 G04 每个 canonical case 都从权威双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_revoke() {
    let trace = G04ScenarioRunner::run_ws_revoke();
    assert_group_created_contract_for(&trace.group_created, &g04_scenario_spec());
}

#[test]
/// 证明 happy path 只在 WS authority PersistOk 后投影 recalled Frame。
fn happy_path_invokes_opaque_action_and_projects_recalled() {
    let trace = G04ScenarioRunner::run_ws_revoke();
    assert_revocable_contract(&trace.revocable);
    assert_recalled_contract(&trace.recalled, Some(g04_scenario_spec().request_id));
}

#[test]
/// 证明 HTTP 基础设施错误不会写入撤回事实或发布伪成功 Frame。
fn infrastructure_failure_keeps_existing_message_visible() {
    let trace = G04ScenarioRunner::run_http_failure();
    assert_revocable_contract(&trace.revocable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 PersistAtomic 失败不部分发布，同一 authority fact 可安全重放。
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G04ScenarioRunner::run_persist_failure_replay();
    assert_revocable_contract(&trace.revocable);
    assert_recalled_contract(
        &trace.recalled_after_replay,
        Some(g04_scenario_spec().request_id),
    );
}

#[test]
/// 证明重复 WS 与迟到 HTTP 不会重复产生撤回 mutation。
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G04ScenarioRunner::run_ws_revoke();
    assert_recalled_contract(&trace.recalled, Some(g04_scenario_spec().request_id));
}

#[test]
/// 证明 cold-start 只凭 durable revoked row 恢复 recalled snapshot。
fn recovery_projects_recalled_without_platform_reducer() {
    let trace = G04ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g04_scenario_spec());
    assert_recalled_contract(&trace.recovered, None);
}
