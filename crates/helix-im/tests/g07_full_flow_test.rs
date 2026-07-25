//! G07 加急与确认的 Helix sans-IO 完整回放合同。

#[path = "support/g07_scenario.rs"]
mod scenario;

use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_urgent_receipts_contract,
    assert_urgent_timeline_contract, assert_urgentable_contract, g07_scenario_spec,
    G07ScenarioRunner, ACTOR_ID, OBSERVER_ID, REQUEST_ID,
};

#[test]
/// 证明 G07 canonical case 从权威双用户 fresh group 与 source post 开始。
fn prerequisite_creates_authoritative_group_before_urgent() {
    let trace = G07ScenarioRunner::run_urgent_post();
    assert_group_created_contract_for(&trace.group_created, &g07_scenario_spec());
}

#[test]
/// 证明发起加急只在 WS authority PersistOk 后发布 active timeline 与 pending receipts。
fn happy_path_projects_active_urgent_and_pending_receipts() {
    let trace = G07ScenarioRunner::run_urgent_post();
    assert_urgentable_contract(&trace.urgentable);
    assert_urgent_timeline_contract(&trace.urgent_active, false, Some(REQUEST_ID));
    assert_urgent_receipts_contract(&trace.receipts_pending, false);
}

#[test]
/// 证明 Observer 的确认 action 收敛为 acknowledged timeline 与 completed receipts。
fn confirm_path_projects_acknowledged_urgent_and_completed_receipts() {
    let trace = G07ScenarioRunner::run_urgent_confirm();
    assert_urgent_timeline_contract(&trace.confirmable, false, None);
    assert_urgent_timeline_contract(
        &trace.urgent_confirmed,
        true,
        Some("g07-wdio-confirm-request-1"),
    );
    assert_urgent_receipts_contract(&trace.receipts_completed, true);
}

#[test]
/// 证明客户端身份和重复 targets 在 Helix command 边界 fail closed。
fn business_failure_rejects_spoofed_actor_and_duplicate_targets() {
    for payload in [
        json!({
            "post_id": g07_scenario_spec().server_id,
            "target_ids": [OBSERVER_ID],
            "user_id": ACTOR_ID,
        }),
        json!({
            "post_id": g07_scenario_spec().server_id,
            "target_ids": [OBSERVER_ID, OBSERVER_ID],
        }),
    ] {
        let bytes = serde_json::to_vec(&payload).expect("invalid urgent payload serializes");
        assert!(
            handle_outbound(
                "im_urgent_post",
                &bytes,
                "http://g07-wdio.invalid",
                "http://g07-wdio.invalid",
                None,
                Correlation::from_raw(700),
            )
            .is_err(),
            "invalid urgent authority input must be rejected"
        );
    }
}

#[test]
/// 证明 HTTP 基础设施失败不持久化或发布伪加急终态。
fn infrastructure_failure_keeps_existing_timeline() {
    let trace = G07ScenarioRunner::run_http_failure();
    assert_urgentable_contract(&trace.urgentable);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 PersistAtomic 失败不部分发布，同一 authority fact 可安全重放。
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G07ScenarioRunner::run_persist_failure_replay();
    assert_urgentable_contract(&trace.urgentable);
    assert_urgent_timeline_contract(&trace.urgent_active_after_replay, false, Some(REQUEST_ID));
}

#[test]
/// 证明 duplicate WS 和 late HTTP 已由场景 runner 校验为无额外 Frame。
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G07ScenarioRunner::run_urgent_post();
    assert_urgent_timeline_contract(&trace.urgent_active, false, Some(REQUEST_ID));
}

#[test]
/// 证明 cold-start 只凭 durable expedite_map 恢复 timeline 与 receipt snapshots。
fn recovery_projects_urgent_without_platform_reducer() {
    let trace = G07ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g07_scenario_spec());
    assert_urgent_timeline_contract(&trace.recovered_timeline, false, None);
    assert_urgent_receipts_contract(&trace.recovered_receipts, false);
}
