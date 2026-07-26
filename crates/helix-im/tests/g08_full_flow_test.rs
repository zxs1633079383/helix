//! G08 创建定时消息的 Helix sans-IO 完整回放合同。

#[path = "support/g08_scenario.rs"]
mod scenario;

use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_schedule_summary_contract, g08_scenario_spec,
    G08ScenarioRunner, REQUEST_ID,
};

#[test]
/// 证明 G08 canonical case 总是从双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_schedule() {
    let trace = G08ScenarioRunner::run_created();
    assert_group_created_contract_for(&trace.group_created, &g08_scenario_spec());
}

#[test]
/// 证明 WS authority 必须跨 PersistAtomic/readback 后才发布 schedule Frame。
fn happy_path_projects_complete_schedule_summary() {
    let trace = G08ScenarioRunner::run_created();
    assert_schedule_summary_contract(&trace.initial_summary, false, None);
    assert_schedule_summary_contract(&trace.scheduled_summary, true, Some(REQUEST_ID));
}

#[test]
/// 证明未知字段和调用方伪造 temporary_id 在 command 边界 fail closed。
fn business_failure_rejects_unfrozen_schedule_fields() {
    for payload in [
        json!({
            "channel_id": g08_scenario_spec().channel_id,
            "message": "scheduled",
            "schedule_post_at": 1_800_000_000_000_i64,
            "temporary_id": "platform-owned"
        }),
        json!({
            "channel_id": g08_scenario_spec().channel_id,
            "message": "scheduled",
            "schedule_post_at": 1_800_000_000_000_i64,
            "local_terminal": true
        }),
    ] {
        let bytes = serde_json::to_vec(&payload).expect("invalid G08 payload serializes");
        assert!(
            handle_outbound(
                "im_create_schedule",
                &bytes,
                "http://g08-wdio.invalid",
                "http://g08-wdio.invalid",
                None,
                Correlation::from_raw(800),
            )
            .is_err(),
            "platform-only G08 fields must be rejected"
        );
    }
}

#[test]
/// 证明 HTTP failure 只结束 transport correlation，不写入或发布 schedule 终态。
fn infrastructure_failure_preserves_unscheduled_summary() {
    let trace = G08ScenarioRunner::run_http_failure();
    assert_schedule_summary_contract(&trace.initial_summary, false, None);
    assert_eq!(trace.emitted_frame_count, 0);
    assert_eq!(trace.persisted_write_count, 0);
}

#[test]
/// 证明 PersistAtomic 失败不部分发布且同一 authority revision 可以安全重放。
fn persistence_failure_is_atomic_and_replayable() {
    let trace = G08ScenarioRunner::run_persist_failure_replay();
    assert_schedule_summary_contract(&trace.initial_summary, false, None);
    assert_schedule_summary_contract(&trace.scheduled_after_replay, true, Some(REQUEST_ID));
}

#[test]
/// 证明 duplicate WS 与 late HTTP 已由 runner 验证为无额外 Frame。
fn authority_race_and_duplicate_are_idempotent() {
    let trace = G08ScenarioRunner::run_created();
    assert_schedule_summary_contract(&trace.scheduled_summary, true, Some(REQUEST_ID));
}

#[test]
/// 证明冷启动只凭 durable channel read model 恢复完整 schedule typed view。
fn recovery_projects_schedule_without_platform_reducer() {
    let trace = G08ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g08_scenario_spec());
    assert_schedule_summary_contract(&trace.recovered_summary, true, None);
}
