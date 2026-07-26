//! G-15a 创建群聊的 Helix sans-IO 完整流程合同。

#[path = "support/g15a_scenario.rs"]
mod scenario;

use scenario::{assert_created_contract, G15aScenarioRunner, REQUEST_ID};

#[test]
/// 证明 HTTP authority 只能在 PersistAtomic 成功后发布完整 summary+roster Frame。
fn happy_path_is_durable_and_field_complete() {
    let trace = G15aScenarioRunner::run_http_created();
    assert!(trace.persist_corr.raw() > 0);
    assert_created_contract(&trace.created, Some(REQUEST_ID));
}

#[test]
/// 证明业务拒绝不伪造本地频道或 terminal Frame。
fn business_failure_is_fail_closed() {
    assert_eq!(G15aScenarioRunner::run_http_failure(true), 0);
}

#[test]
/// 证明 transport failure 不写入 channel/member，也不发布 terminal Frame。
fn infrastructure_failure_is_fail_closed() {
    assert_eq!(G15aScenarioRunner::run_http_failure(false), 0);
}

#[test]
/// 证明持久失败不部分发布且相同 WS authority 可以重新提交。
fn persistence_failure_is_atomic_and_replayable() {
    let frame = G15aScenarioRunner::run_persist_failure_replay();
    assert_created_contract(&frame, Some(REQUEST_ID));
}

#[test]
/// 证明 HTTP/WS 重复和迟到回包只产生一次已提交终态。
fn authority_race_is_idempotent() {
    let trace = G15aScenarioRunner::run_http_created();
    assert_created_contract(&trace.created, Some(REQUEST_ID));
}

#[test]
/// 证明冷启动从 durable rows 恢复 typed summary 和 roster，不依赖平台 reducer。
fn recovery_rebuilds_both_typed_slots() {
    let trace = G15aScenarioRunner::run_recovery();
    assert!(trace.summary.mutations.iter().all(|mutation| {
        mutation.mutation.surface == helix_im::interaction_frame::SurfaceId::MessageChannelSummary
    }));
    assert!(trace.roster.mutations.iter().all(|mutation| {
        mutation.mutation.surface == helix_im::interaction_frame::SurfaceId::MessageMemberRoster
    }));
    assert!(trace
        .summary
        .mutations
        .iter()
        .chain(trace.roster.mutations.iter())
        .all(|mutation| mutation.mutation.causation_id.is_none()));
}
