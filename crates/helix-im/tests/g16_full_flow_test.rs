//! G16 关闭/退出群的 Helix sans-IO 完整流程合同。

#[path = "support/g15a_scenario.rs"]
mod prerequisite;
#[path = "support/g16_scenario.rs"]
mod scenario;

use scenario::{assert_lifecycle_contract, G16ScenarioRunner};

#[test]
/// 证明每个 G16 canonical 验收先复用生产 G15a fresh-group authority，而非遗留数据。
fn prerequisite_creates_fresh_group_before_lifecycle_command() {
    let created = prerequisite::G15aScenarioRunner::run_http_created();
    prerequisite::assert_created_contract(&created.created, Some(prerequisite::REQUEST_ID));
}

#[test]
/// 证明 owner close 必须跨 PersistAtomic 后才发布 closed Frame。
fn close_authority_is_durable_and_field_complete() {
    let trace = G16ScenarioRunner::run_closed();
    assert_lifecycle_contract(&trace.initial, "active");
    assert_lifecycle_contract(&trace.terminal, "closed");
    assert!(trace.persist_corr.raw() > 0);
}

#[test]
/// 证明 self-leave HTTP SUCCESS 仍需本地 PersistOk 才发布 left Frame。
fn leave_authority_is_durable_and_distinct_from_close() {
    let trace = G16ScenarioRunner::run_left();
    assert_lifecycle_contract(&trace.initial, "active");
    assert_lifecycle_contract(&trace.terminal, "left");
    assert!(trace.persist_corr.raw() > 0);
}

#[test]
/// 证明 transport 与 business failure 都不会伪造 leave 终态。
fn leave_failures_are_fail_closed() {
    assert_eq!(G16ScenarioRunner::run_failure(true, false), 0);
    assert_eq!(G16ScenarioRunner::run_failure(false, false), 0);
}

#[test]
/// 证明 close 本地持久失败不会发布部分 lifecycle Frame。
fn close_persist_failure_is_fail_closed() {
    assert_eq!(G16ScenarioRunner::run_failure(false, true), 0);
}

#[test]
/// 证明 cold recovery 能从 durable close/leave 事实恢复不同生命周期。
fn recovery_preserves_close_and_leave_semantics() {
    assert_lifecycle_contract(&G16ScenarioRunner::run_recovery(false), "closed");
    assert_lifecycle_contract(&G16ScenarioRunner::run_recovery(true), "left");
}

#[test]
/// 证明重复或迟到的已消费 Persist correlation 不会生成第二个终态。
fn terminal_correlations_are_single_use() {
    let closed = G16ScenarioRunner::run_closed();
    let left = G16ScenarioRunner::run_left();
    assert_eq!(closed.late_frame_count, 0);
    assert_eq!(left.late_frame_count, 0);
}
