//! G06 快捷回复 emoji 的 Helix sans-IO 完整回放合同。

#[path = "support/g06_scenario.rs"]
mod scenario;

use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_reactable_contract, assert_reacted_contract,
    g06_scenario_spec, G06ScenarioRunner,
};

#[test]
/// 证明 G06 每个 canonical case 都从权威双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_reaction() {
    let trace = G06ScenarioRunner::run_ws_reaction();
    assert_group_created_contract_for(&trace.group_created, &g06_scenario_spec());
}

#[test]
/// 证明 happy path 只在 WS authority PersistOk 后投影 reacted Frame。
fn happy_path_invokes_opaque_action_and_projects_reaction() {
    let trace = G06ScenarioRunner::run_ws_reaction();
    assert_reactable_contract(&trace.reactable);
    assert_reacted_contract(&trace.reacted, Some(g06_scenario_spec().request_id));
}

#[test]
/// 证明客户端提交 user_id 的非 canonical command 在 Helix 边界被拒绝。
fn business_failure_rejects_client_supplied_actor_identity() {
    let payload = serde_json::to_vec(&json!({
        "post_id": g06_scenario_spec().server_id,
        "emoji": ":thumbsup:",
        "user_id": g06_scenario_spec().actor_id,
    }))
    .expect("invalid reaction payload serializes");
    let result = handle_outbound(
        "im_quick_reply",
        &payload,
        "http://g06-wdio.invalid",
        "http://g06-wdio.invalid",
        None,
        Correlation::from_raw(600),
    );
    assert!(
        result.is_err(),
        "actor identity must come from the authenticated session"
    );
}

#[test]
/// 证明 HTTP 基础设施错误不会写入 reaction 事实或发布伪成功 Frame。
fn infrastructure_failure_keeps_existing_reaction_state() {
    let trace = G06ScenarioRunner::run_http_failure();
    assert_reactable_contract(&trace.reactable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 PersistAtomic 失败不部分发布，同一 authority fact 可安全重放。
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G06ScenarioRunner::run_persist_failure_replay();
    assert_reactable_contract(&trace.reactable);
    assert_reacted_contract(
        &trace.reacted_after_replay,
        Some(g06_scenario_spec().request_id),
    );
}

#[test]
/// 证明重复 WS 与迟到 HTTP 不会重复产生 reaction mutation。
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G06ScenarioRunner::run_ws_reaction();
    assert_reacted_contract(&trace.reacted, Some(g06_scenario_spec().request_id));
}

#[test]
/// 证明 cold-start 只凭 durable reaction row 恢复 reacted snapshot。
fn recovery_projects_reaction_without_platform_reducer() {
    let trace = G06ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g06_scenario_spec());
    assert_reacted_contract(&trace.recovered, None);
}
