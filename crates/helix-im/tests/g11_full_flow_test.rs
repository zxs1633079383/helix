//! G11 按 postId 定位的 Helix sans-IO 完整生产合同。

#[path = "support/g11_scenario.rs"]
mod scenario;

use serde_json::Value;

use scenario::{
    assert_group_created_contract_for, assert_located_contract, g11_scenario_spec,
    G11ScenarioRunner, REQUEST_ID, TARGET_ID,
};

/// 证明 canonical G11 case 从双用户 fresh group 和完整 bounded timeline 开始。
#[test]
fn prerequisite_creates_fresh_group_and_attaches_target_timeline() {
    let trace = G11ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &g11_scenario_spec());
    assert_eq!(trace.initial.mutations.len(), 1);
    assert_located_contract(&trace.located);
}

/// 证明 canonical command 只向 posts/get 发送 postIds，reqId 留在本地 correlation。
#[test]
fn command_admission_preserves_wire_and_local_correlation_boundary() {
    let trace = G11ScenarioRunner::run_success();
    let body: Value = serde_json::from_slice(
        trace
            .request
            .body
            .as_ref()
            .expect("G11 request has JSON body")
            .as_ref(),
    )
    .expect("G11 wire body is JSON");
    assert_eq!(body, serde_json::json!({"postIds": [TARGET_ID]}));
    assert!(!serde_json::to_string(&body)
        .expect("wire body serializes")
        .contains(REQUEST_ID));
}

/// 证明 HTTP authority 只发布一个完整 V2 Frame，且 target 与 directive 原子同行。
#[test]
fn happy_path_projects_target_and_reveal_in_one_frame() {
    let trace = G11ScenarioRunner::run_success();
    assert_located_contract(&trace.located);
}

/// 证明 transport failure 不发布 raw event、空成功 Frame 或额外持久写。
#[test]
fn infrastructure_failure_preserves_last_good_timeline() {
    let trace = G11ScenarioRunner::run_http_failure();
    assert_eq!(trace.initial.mutations.len(), 1);
    assert_eq!(trace.effect_count, 0);
}

/// 证明不在当前 bounded window 的目标不会被插入成孤立消息。
#[test]
fn absent_target_fails_closed_without_client_side_insertion() {
    let trace = G11ScenarioRunner::run_absent_target();
    assert_eq!(trace.effect_count, 0);
}

/// 证明 consumed correlation 的重复 reply 不产生第二终态 Frame。
#[test]
fn duplicate_reply_is_idempotent() {
    let trace = G11ScenarioRunner::run_duplicate_reply();
    assert_located_contract(trace.retry.as_ref().expect("first locate succeeded"));
    assert_eq!(trace.effect_count, 0);
}

/// 证明一次失败不会污染新 correlation，后续合法 locate 可恢复。
#[test]
fn recovery_retries_with_new_correlation_and_reaches_same_contract() {
    let trace = G11ScenarioRunner::run_recovery();
    assert_located_contract(trace.retry.as_ref().expect("retry locate succeeded"));
}
