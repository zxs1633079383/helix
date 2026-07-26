//! G13 会话已读完整 sans-IO 流程合同。

#[path = "support/g13_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_initial_contract, assert_terminal_contract,
    g13_scenario_spec, G13ScenarioRunner, CHANNEL_ID,
};

/// 证明 G13 每次先经过可复用 fresh-group prerequisite。
#[test]
fn g13_reuses_fresh_group_prerequisite() {
    let trace = G13ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &g13_scenario_spec());
}

/// 证明 command wire 不提交 viewer 身份或未读结果冒充 authority。
#[test]
fn g13_command_admission_preserves_server_authority() {
    let trace = G13ScenarioRunner::run_success();
    assert_eq!(trace.request.method, "POST");
    assert!(trace.request.url.ends_with("/channels/view"));
    let body: serde_json::Value = serde_json::from_slice(
        trace
            .request
            .body
            .as_ref()
            .expect("G13 request carries a JSON body"),
    )
    .expect("G13 request body decodes");
    assert_eq!(
        body,
        serde_json::json!({"channels":[{"id":CHANNEL_ID,"isRoot":true}]})
    );
    assert!(body.get("userId").is_none());
    assert!(body.get("unreadCount").is_none());
}

/// 证明 update_channel 只有在 matching PersistOk 后才发布 paired terminal Frame。
#[test]
fn g13_happy_path_is_durable_and_atomically_paired() {
    let trace = G13ScenarioRunner::run_success();
    assert!(trace.persist_corr.raw() > 0);
    assert_initial_contract(&trace.initial);
    assert_terminal_contract(&trace.terminal);
}

/// 证明 storage failure 不产生伪成功 Frame。
#[test]
fn g13_persist_failure_keeps_existing_unread_state() {
    let trace = G13ScenarioRunner::run_persist_failure();
    assert_initial_contract(&trace.initial);
    assert_eq!(trace.frame_count, 0);
}

/// 证明重复 authority 和迟到 PersistOk 均保持幂等。
#[test]
fn g13_duplicate_and_late_authority_do_not_repeat_terminal_frame() {
    let duplicate = G13ScenarioRunner::run_duplicate_authority();
    let late = G13ScenarioRunner::run_late_persist_reply();
    assert_eq!(duplicate.frame_count, 0);
    assert_eq!(late.frame_count, 0);
}

/// 证明 cold recovery 从 durable row 重建相同已读 typed snapshot。
#[test]
fn g13_recovery_rebuilds_read_state_without_platform_cache() {
    let recovered = G13ScenarioRunner::run_recovery();
    recovered.validate().expect("G13 recovery Frame is valid");
    assert_eq!(recovered.mutations.len(), 2);
    let serialized = serde_json::to_value(recovered).expect("G13 recovery Frame serializes");
    assert_eq!(
        serialized["mutations"][0]["mutation"]["view"]["sections"][0]["items"][0]["unreadCount"],
        0
    );
    assert_eq!(
        serialized["mutations"][1]["mutation"]["view"]["unreadTotal"],
        0
    );
}
