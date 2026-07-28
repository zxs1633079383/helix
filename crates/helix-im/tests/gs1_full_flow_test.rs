//! GS1 hello 全量增量的 Helix sans-IO 完整流程合同。

#[path = "support/gs1_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_terminal_contract, gs1_scenario_spec,
    Gs1ScenarioRunner,
};

#[test]
/// 证明 GS1 每次仍先经过可复用 fresh-group prerequisite。
fn prerequisite_creates_a_fresh_group() {
    let trace = Gs1ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &gs1_scenario_spec());
}

#[test]
/// 证明 hello 只发一次 bootstrap increment HTTP 且不提交平台业务字段。
fn hello_admission_is_single_and_server_authoritative() {
    let trace = Gs1ScenarioRunner::run_success();
    assert_eq!(trace.increment_request.method, "POST");
    assert!(trace
        .increment_request
        .url
        .ends_with("/channels/load/increment"));
    let body: serde_json::Value = serde_json::from_slice(
        trace
            .increment_request
            .body
            .as_ref()
            .expect("GS1 increment request carries body"),
    )
    .expect("GS1 increment request decodes");
    assert_eq!(body["timestamp"], 0);
    assert!(body["cursors"].is_array());
    assert!(body.get("channels").is_none());
}

#[test]
/// 证明 global end 合批 channel/member/cursor，最终 Frame 等待 PersistOk 和 scan。
fn happy_path_is_durable_and_field_complete() {
    let trace = Gs1ScenarioRunner::run_success();
    assert!(trace.persist_corr.raw() > 0);
    let channel = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel")
        .expect("GS1 persists channel");
    let member = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel_member")
        .expect("GS1 persists members");
    let cursor = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel_event_cursor")
        .expect("GS1 persists cursor");
    assert!(channel < member && member < cursor);
    assert_terminal_contract(&trace.terminal);
}

#[test]
/// 证明非法业务帧与本地持久失败均不会形成最终 Frame 或 refresh scan。
fn business_and_infrastructure_failures_are_fail_closed() {
    let invalid = Gs1ScenarioRunner::run_invalid_increment();
    assert_eq!(invalid.frame_count, 0);
    assert_eq!(invalid.refresh_scan_count, 0);
    let persist = Gs1ScenarioRunner::run_persist_failure();
    assert_eq!(persist.frame_count, 0);
    assert_eq!(persist.refresh_scan_count, 0);
}

#[test]
/// 证明已消费的迟到 PersistOk 不重复触发 scan 或 Frame。
fn late_persist_reply_is_idempotent() {
    let late = Gs1ScenarioRunner::run_late_persist_reply();
    assert_eq!(late.frame_count, 0);
    assert_eq!(late.refresh_scan_count, 0);
}

#[test]
/// 证明恢复可从 durable row 重建同一 typed dialog snapshot。
fn recovery_rebuilds_from_durable_channel_rows() {
    assert_terminal_contract(&Gs1ScenarioRunner::run_recovery());
}
