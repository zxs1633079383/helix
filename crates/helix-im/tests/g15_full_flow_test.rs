//! G-15 创建话题的 Helix sans-IO 完整流程合同。

#[path = "support/g15_scenario.rs"]
mod scenario;

use scenario::{assert_summary_contract, assert_thread_contract, G15ScenarioRunner};

#[test]
/// 证明 HTTP authority 只能在 relation PersistOk 与 durable root readback 后发布 typed Frame。
fn happy_path_is_durable_and_field_complete() {
    let trace = G15ScenarioRunner::run_http_created();
    assert!(trace.relation_persist_corr.raw() > 0);
    assert!(trace.root_read_corr.raw() > trace.relation_persist_corr.raw());
    assert_thread_contract(&trace.thread);
}

#[test]
/// 证明业务拒绝不写 relation，也不发布 terminal Frame。
fn business_failure_is_fail_closed() {
    assert_eq!(G15ScenarioRunner::run_http_failure(true), 1);
}

#[test]
/// 证明 transport failure 不写 relation，也不发布 terminal Frame。
fn infrastructure_failure_is_fail_closed() {
    assert_eq!(G15ScenarioRunner::run_http_failure(false), 1);
}

#[test]
/// 证明 relation 持久失败不会进入 durable readback 或发布部分 Frame。
fn persistence_failure_is_fail_closed() {
    assert_eq!(G15ScenarioRunner::run_persist_failure(), 0);
}

#[test]
/// 证明迟到的已消费 read correlation 不会重复发布 terminal Frame。
fn late_reply_is_idempotent() {
    let trace = G15ScenarioRunner::run_http_created();
    assert_thread_contract(&trace.thread);
}

#[test]
/// 证明 cold recovery 从 durable topic channel row 重建 typed summary。
fn recovery_rebuilds_topic_summary() {
    let trace = G15ScenarioRunner::run_recovery();
    assert_summary_contract(&trace.summary);
}
