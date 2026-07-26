//! G09 取消定时的 Helix sans-IO 完整回放合同。

#[path = "support/g09_scenario.rs"]
mod scenario;

use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::json;

use scenario::{
    assert_canceled_summary_contract, assert_group_created_contract_for,
    assert_scheduled_prerequisite_contract, g09_scenario_spec, G09ScenarioRunner, REQUEST_ID,
};

/// 证明 G09 canonical case 从双用户 fresh group 和真实 G08 排程开始。
#[test]
fn prerequisite_creates_group_and_schedule_before_cancel() {
    let trace = G09ScenarioRunner::run_canceled();
    assert_group_created_contract_for(&trace.group_created, &g09_scenario_spec());
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
}

/// 证明 cancel authority 跨 PersistAtomic/readback 后才发布完整 terminal Frame。
#[test]
fn happy_path_projects_complete_canceled_summary() {
    let trace = G09ScenarioRunner::run_canceled();
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
    assert_canceled_summary_contract(&trace.canceled_summary, Some(REQUEST_ID));
}

/// 证明未知字段和平台伪造终态在 G09 command 边界 fail closed。
#[test]
fn business_failure_rejects_unfrozen_cancel_fields() {
    for payload in [
        json!({
            "channel_id": g09_scenario_spec().channel_id,
            "local_terminal": "canceled"
        }),
        json!({
            "channel_id": g09_scenario_spec().channel_id,
            "schedule_revision": 99
        }),
    ] {
        let bytes = serde_json::to_vec(&payload).expect("invalid G09 payload serializes");
        assert!(
            handle_outbound(
                "im_cancel_schedule",
                &bytes,
                "http://g09-wdio.invalid",
                "http://g09-wdio.invalid",
                None,
                Correlation::from_raw(900),
            )
            .is_err(),
            "platform-only G09 fields must be rejected"
        );
    }
}

/// 证明 HTTP failure 只结束 transport correlation，不覆盖已排程事实。
#[test]
fn infrastructure_failure_preserves_scheduled_summary() {
    let trace = G09ScenarioRunner::run_http_failure();
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

/// 证明 PersistAtomic 失败不部分发布且同一 cancel revision 可以安全重放。
#[test]
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G09ScenarioRunner::run_persist_failure_replay();
    assert_scheduled_prerequisite_contract(&trace.scheduled_summary);
    assert_canceled_summary_contract(&trace.canceled_after_replay, Some(REQUEST_ID));
}

/// 证明 duplicate cancel、迟到 create 和迟到 HTTP 均不产生额外 terminal Frame。
#[test]
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G09ScenarioRunner::run_canceled();
    assert_canceled_summary_contract(&trace.canceled_summary, Some(REQUEST_ID));
}

/// 证明冷启动只凭 durable canceled read model 恢复完整 typed view。
#[test]
fn recovery_projects_canceled_state_without_platform_reducer() {
    let trace = G09ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g09_scenario_spec());
    assert_canceled_summary_contract(&trace.recovered_summary, None);
}
