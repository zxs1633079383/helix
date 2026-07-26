//! G12 回复列表/分支的 Helix sans-IO 完整生产合同。

#[path = "support/g12_scenario.rs"]
mod scenario;

use serde_json::Value;

use scenario::{
    assert_branch_contract, assert_empty_branch_contract, assert_group_created_contract_for,
    assert_snapshot_contract, g12_scenario_spec, G12ScenarioRunner, FIRST_REPLY_ID, ROOT_ID,
};

/// 证明 canonical G12 case 从双用户 fresh group 开始。
#[test]
fn prerequisite_creates_fresh_group_before_reply_queries() {
    let trace = G12ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &g12_scenario_spec());
}

/// 证明两个 command 只发送服务端 wire 字段，本地 correlation 不泄漏。
#[test]
fn command_admission_preserves_wire_and_local_correlation_boundary() {
    let trace = G12ScenarioRunner::run_success();
    let snapshot: Value = serde_json::from_slice(
        trace
            .snapshot_request
            .body
            .as_ref()
            .expect("snapshot request has body")
            .as_ref(),
    )
    .expect("snapshot wire body is JSON");
    assert_eq!(
        snapshot,
        serde_json::json!({"replyId": ROOT_ID, "pageNumber": 1, "pageSize": 20})
    );
    let branch: Value = serde_json::from_slice(
        trace
            .branch_request
            .body
            .as_ref()
            .expect("branch request has body")
            .as_ref(),
    )
    .expect("branch wire body is JSON");
    assert_eq!(
        branch,
        serde_json::json!({
            "replyFirstLevelId": FIRST_REPLY_ID,
            "pageNumber": 1,
            "pageSize": 20,
        })
    );
}

/// 证明 snapshot authority 直接发布完整 typed Frame 而非 raw reply event。
#[test]
fn happy_path_projects_complete_thread_snapshot() {
    let trace = G12ScenarioRunner::run_success();
    assert_snapshot_contract(&trace.snapshot);
}

/// 证明 attached branch 以完整 replace-view patch 原子追加节点。
#[test]
fn branch_path_projects_complete_thread_patch() {
    let trace = G12ScenarioRunner::run_success();
    assert_branch_contract(&trace.branch);
}

/// 证明 attached 空分支也发布 causal terminal Frame，而不是让平台永久等待。
#[test]
fn empty_branch_authority_settles_with_complete_thread_patch() {
    assert_empty_branch_contract(&G12ScenarioRunner::run_empty_branch());
}

/// 证明 transport failure 不发布空成功 Frame。
#[test]
fn infrastructure_failure_preserves_last_good_thread() {
    assert_eq!(G12ScenarioRunner::run_http_failure().frame_count, 0);
}

/// 证明 mixed-channel authority 整批失败而不泄漏跨群节点。
#[test]
fn business_authority_mismatch_fails_closed() {
    assert_eq!(G12ScenarioRunner::run_mixed_authority().frame_count, 0);
}

/// 证明 consumed correlation 的重复 reply 不产生第二终态。
#[test]
fn duplicate_reply_is_idempotent() {
    let trace = G12ScenarioRunner::run_duplicate_reply();
    assert_snapshot_contract(trace.recovered.as_ref().expect("first reply succeeded"));
    assert_eq!(trace.frame_count, 0);
}

/// 证明 detached branch 保持 view-silent，后续 snapshot 可恢复完整线程。
#[test]
fn detached_branch_recovers_via_authoritative_snapshot() {
    let trace = G12ScenarioRunner::run_detached_recovery();
    assert_eq!(trace.frame_count, 0);
    assert_snapshot_contract(trace.recovered.as_ref().expect("snapshot recovered"));
}
