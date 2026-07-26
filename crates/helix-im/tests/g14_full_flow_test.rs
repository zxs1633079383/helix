//! G14 单条已读完整 sans-IO 流程合同。

#[path = "support/g14_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_receipt_contract, g14_scenario_spec,
    G14ScenarioRunner, CHANNEL_ID, POST_ID,
};

/// 证明 G14 每次先经过可复用 fresh-group prerequisite。
#[test]
fn g14_reuses_fresh_group_prerequisite() {
    let trace = G14ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &g14_scenario_spec());
}

/// 证明 direct command 只提交 canonical channelId/posts，不提交已读结果。
#[test]
fn g14_command_admission_preserves_server_authority() {
    let trace = G14ScenarioRunner::run_success();
    assert!(trace.persist_corr.raw() > 0);
    assert!(trace.receipt_corr.raw() > trace.persist_corr.raw());
    assert_eq!(trace.request.method, "POST");
    assert!(trace.request.url.ends_with("/post/read"));
    let body: serde_json::Value = serde_json::from_slice(
        trace
            .request
            .body
            .as_ref()
            .expect("G14 request carries a JSON body"),
    )
    .expect("G14 request body decodes");
    assert_eq!(
        body,
        serde_json::json!({"channelId":CHANNEL_ID,"posts":[POST_ID]})
    );
    assert!(body.get("readBits").is_none());
    assert!(body.get("userId").is_none());
}

/// 证明 WS authority 只有在 PersistOk 与 receipt authority 回包后才发布终态 Frame。
#[test]
fn g14_happy_path_is_durable_and_typed() {
    let trace = G14ScenarioRunner::run_success();
    assert_receipt_contract(&trace.initial, 1, false);
    assert_receipt_contract(&trace.terminal, 2, true);
}

/// 证明 HTTP 失败只是 transport failure，不产生伪成功 Frame。
#[test]
fn g14_business_failure_keeps_existing_receipt_state() {
    let trace = G14ScenarioRunner::run_command_failure();
    assert_receipt_contract(&trace.initial, 1, false);
    assert_eq!(trace.frame_count, 0);
    assert_eq!(trace.persist_count, 0);
    assert_eq!(trace.receipt_http_count, 0);
}

/// 证明 storage failure 阻断 raw event、refresh 与 typed Frame。
#[test]
fn g14_persist_failure_keeps_existing_receipt_state() {
    let trace = G14ScenarioRunner::run_persist_failure();
    assert_receipt_contract(&trace.initial, 1, false);
    assert_eq!(trace.frame_count, 0);
    assert_eq!(trace.receipt_http_count, 0);
}

/// 证明重复 authority 与迟到 PortReply 均保持无重复写和无重复 Frame。
#[test]
fn g14_duplicate_and_late_authority_are_idempotent() {
    let duplicate = G14ScenarioRunner::run_duplicate_authority();
    let late = G14ScenarioRunner::run_late_persist_reply();
    assert_eq!(duplicate.frame_count, 0);
    assert_eq!(duplicate.persist_count, 0);
    assert_eq!(duplicate.receipt_http_count, 0);
    assert_eq!(late.frame_count, 0);
    assert_eq!(late.persist_count, 0);
    assert_eq!(late.receipt_http_count, 0);
}

/// 证明 cold attach 从 authority snapshot 重建相同 typed receipt view。
#[test]
fn g14_recovery_rebuilds_receipts_without_platform_cache() {
    let recovered = G14ScenarioRunner::run_recovery();
    assert_receipt_contract(&recovered, 1, true);
}
