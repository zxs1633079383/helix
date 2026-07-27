//! GS2 离线 sync notify 的 Helix sans-IO 完整流程合同。

#[path = "support/gs2_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_terminal_contract, gs2_scenario_spec,
    Gs2ScenarioRunner,
};

#[test]
/// 证明 GS2 每次仍先经过可复用 fresh-group prerequisite。
fn prerequisite_creates_a_fresh_group() {
    let trace = Gs2ScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &gs2_scenario_spec());
}

#[test]
/// 证明 explicit command 只提交频道意图，实际 fromSeq 由 Helix cursor 决定。
fn admission_uses_helix_cursor_and_sync_notify() {
    let trace = Gs2ScenarioRunner::run_success();
    assert!(trace.request.url.ends_with("/channel/sync/notify"));
    let body: serde_json::Value = serde_json::from_slice(
        trace
            .request
            .body
            .as_ref()
            .expect("GS2 sync request has body"),
    )
    .expect("GS2 sync request decodes");
    assert_eq!(body["cursors"][0]["channelId"], scenario::CHANNEL_ID);
    assert_eq!(body["cursors"][0]["fromSeq"], 0);
    assert_ne!(body["cursors"][0]["fromSeq"], 999_999);
}

#[test]
/// 证明 message/ledger/coverage/cursor 同批持久后才形成字段完整 Frame。
fn happy_path_is_atomic_and_field_complete() {
    let trace = Gs2ScenarioRunner::run_success();
    let message = trace
        .persist_tables
        .iter()
        .position(|table| *table == "message")
        .expect("GS2 persists message");
    let ledger = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel_event_ledger")
        .expect("GS2 persists ledger");
    let coverage = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel_sync_coverage")
        .expect("GS2 persists coverage");
    let cursor = trace
        .persist_tables
        .iter()
        .position(|table| *table == "channel_event_cursor")
        .expect("GS2 persists cursor");
    assert!(message < ledger && ledger < coverage && coverage < cursor);
    assert_terminal_contract(&trace.terminal);
}

#[test]
/// 证明 HTTP 与本地事务失败都不会形成伪成功 Frame 或 refresh。
fn transport_and_persist_failures_are_fail_closed() {
    let http = Gs2ScenarioRunner::run_http_failure();
    assert_eq!(http.frame_count, 0);
    assert_eq!(http.refresh_count, 0);
    let persist = Gs2ScenarioRunner::run_persist_failure();
    assert_eq!(persist.frame_count, 0);
    assert_eq!(persist.refresh_count, 0);
}

#[test]
/// 证明重复迟到 PersistOk 不会二次 refresh 或重复 mutation。
fn late_persist_reply_is_idempotent() {
    let late = Gs2ScenarioRunner::run_late_persist_reply();
    assert_eq!(late.frame_count, 0);
    assert_eq!(late.refresh_count, 0);
}

#[test]
/// 证明重启 attach 可从 durable channel row 重建同一 typed view。
fn recovery_rebuilds_from_durable_rows() {
    assert_terminal_contract(&Gs2ScenarioRunner::run_recovery());
}
