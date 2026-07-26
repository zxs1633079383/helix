//! G14a 模板已收到的 Helix sans-IO 完整回放合同。

#[path = "support/g14a_scenario.rs"]
mod scenario;

use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_receivable_contract, assert_received_contract,
    g14a_scenario_spec, G14AScenarioRunner,
};

/// 证明 G14a 每个 canonical case 都从权威双用户 fresh group 开始。
#[test]
fn prerequisite_creates_authoritative_group_before_template_receipt() {
    let trace = G14AScenarioRunner::run_ws_received();
    assert_group_created_contract_for(&trace.group_created, &g14a_scenario_spec());
}

/// 证明 happy path 只在 WS post_update PersistOk 后投影 received Frame。
#[test]
fn happy_path_waits_for_durable_template_authority() {
    let trace = G14AScenarioRunner::run_ws_received();
    assert_receivable_contract(&trace.receivable);
    assert_received_contract(&trace.received);
}

/// 证明空 postId 在 Helix command admission 边界被拒绝。
#[test]
fn business_failure_rejects_empty_post_id() {
    let payload =
        serde_json::to_vec(&json!({"post_id": ""})).expect("invalid template payload serializes");
    let result = handle_outbound(
        "im_template_received",
        &payload,
        "http://g14a-wdio.invalid",
        "http://g14a-wdio.invalid",
        None,
        Correlation::from_raw(1_400),
    );
    assert!(result.is_err(), "empty postId must fail closed");
}

/// 证明 HTTP 基础设施错误不会写入 template authority 或发布伪成功 Frame。
#[test]
fn infrastructure_failure_keeps_existing_template_state() {
    let trace = G14AScenarioRunner::run_http_failure();
    assert_receivable_contract(&trace.receivable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

/// 证明 PersistAtomic 失败不部分发布，同一 authority fact 可安全重放。
#[test]
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G14AScenarioRunner::run_persist_failure_replay();
    assert_receivable_contract(&trace.receivable);
    assert_received_contract(&trace.received_after_replay);
}

/// 证明重复 WS 与迟到 HTTP 不会重复产生 template mutation。
#[test]
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G14AScenarioRunner::run_ws_received();
    assert_received_contract(&trace.received);
}

/// 证明 cold-start 只凭 durable template row 恢复 received snapshot。
#[test]
fn recovery_projects_template_state_without_platform_reducer() {
    let trace = G14AScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g14a_scenario_spec());
    assert_received_contract(&trace.recovered);
}
