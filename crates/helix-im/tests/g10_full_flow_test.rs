//! G10 切群首屏的 Helix sans-IO 完整回放合同。

#[path = "support/g10_scenario.rs"]
mod scenario;

use helix_im::interaction_frame::SurfaceLoadState;
use helix_im::query::parse_message_query;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_timeline_contract, g10_scenario_spec,
    G10ScenarioRunner,
};

/// 证明 canonical G10 case 从双用户 fresh group 开始并复用权威 channelId。
#[test]
fn prerequisite_creates_fresh_group_before_timeline_query() {
    let trace = G10ScenarioRunner::run_success_and_recovery();
    assert_group_created_contract_for(&trace.group_created, &g10_scenario_spec());
    assert_timeline_contract(
        &trace.ready,
        SurfaceLoadState::Ready,
        Some("g10-latest-marker"),
        true,
    );
}

/// 证明远端首屏必须先写 cache，再发布完整 ready Frame。
#[test]
fn happy_path_waits_for_cache_barrier_and_projects_complete_window() {
    let trace = G10ScenarioRunner::run_success_and_recovery();
    assert!(trace.remote_cache_persisted_before_ready);
    assert_timeline_contract(
        &trace.ready,
        SurfaceLoadState::Ready,
        Some("g10-latest-marker"),
        true,
    );
}

/// 证明非法 window token 在生产 command 边界 fail closed。
#[test]
fn business_failure_rejects_empty_window_token() {
    let error = parse_message_query(
        &serde_json::to_vec(&json!({
            "channel_id": g10_scenario_spec().channel_id,
            "limit": 20,
            "windowToken": "",
        }))
        .expect("G10 invalid command serializes"),
    )
    .expect_err("empty window token must be rejected");
    assert!(error.to_string().contains("must not be empty"));
}

/// 证明 HTTP failure 只产生 failed Frame，不写空成功事实。
#[test]
fn infrastructure_failure_projects_failed_window_without_persist_write() {
    let trace = G10ScenarioRunner::run_http_failure();
    assert_timeline_contract(&trace.failed, SurfaceLoadState::Failed, None, false);
    assert_eq!(trace.persisted_write_count, 0);
}

/// 证明 cache 失败不建立 coverage，下次查询仍必须回源。
#[test]
fn cache_persist_failure_keeps_remote_view_but_forces_follow_up_http() {
    let trace = G10ScenarioRunner::run_cache_persist_failure();
    assert_timeline_contract(
        &trace.ready_after_cache_failure,
        SurfaceLoadState::Ready,
        Some("g10-cache-failure"),
        false,
    );
    assert_eq!(trace.follow_up_http_count, 1);
}

/// 证明同 scope 最新代际获胜，迟到与重复 reply 都无副作用。
#[test]
fn race_and_idempotency_drop_late_and_duplicate_replies() {
    let trace = G10ScenarioRunner::run_latest_query_wins();
    assert_eq!(trace.late_effect_count, 0);
    assert_eq!(trace.duplicate_effect_count, 0);
    assert_timeline_contract(
        &trace.current,
        SurfaceLoadState::Ready,
        Some("g10-current"),
        true,
    );
}

/// 证明 durable coverage 恢复走本地增量并保持 scope，且不再发 HTTP。
#[test]
fn recovery_uses_local_coverage_and_preserves_scope() {
    let trace = G10ScenarioRunner::run_success_and_recovery();
    assert_eq!(trace.recovery_http_count, 0);
    assert_timeline_contract(
        &trace.recovered,
        SurfaceLoadState::Ready,
        Some("g10-latest-marker"),
        true,
    );
    assert!(matches!(
        trace.recovered.mutations[0].mutation.mutation,
        helix_im::interaction_frame::TypedSurfaceMutation::TimelineDelta { .. }
    ));
}

/// 证明 G10 command admission 保留 Helix 自有 scope 与有界 limit。
#[test]
fn command_admission_preserves_canonical_scope() {
    let request = parse_message_query(
        &serde_json::to_vec(&json!({
            "channel_id": g10_scenario_spec().channel_id,
            "limit": 20,
            "windowToken": scenario::WINDOW_TOKEN,
        }))
        .expect("G10 command serializes"),
    )
    .expect("G10 command is admitted");
    assert_eq!(request.channel_id.as_str(), g10_scenario_spec().channel_id);
    assert_eq!(request.limit, 20);
    assert_eq!(request.window_token, scenario::WINDOW_TOKEN);
}
