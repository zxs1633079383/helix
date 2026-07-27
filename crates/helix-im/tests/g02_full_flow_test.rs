//! G02 固定文档 URL 消息卡片的 Helix sans-IO 完整回放合同。

#[path = "support/g02_scenario.rs"]
mod scenario;

use helix_im::interaction_frame::MessageContentVm;
use scenario::{
    assert_dialog_contract_for, assert_document_timeline_contract,
    assert_group_created_contract_for, assert_recovered_document_timeline_contract, first_message,
    g02_scenario_spec, G02ScenarioRunner, Terminal, DOCUMENT_URL, SERVER_ID,
};

#[test]
/// 证明 G02 每个 case 都从权威双用户 fresh group 开始。
fn prerequisite_creates_authoritative_group_before_document_send() {
    let trace = G02ScenarioRunner::run_http(Terminal::Sent);
    assert_group_created_contract_for(&trace.group_created, &g02_scenario_spec());
}

#[test]
/// 证明固定文档 URL 经 Persist barrier 形成 sending 与 sent typed Document Frames。
fn happy_path_projects_sending_then_sent_document_card() {
    let trace = G02ScenarioRunner::run_http(Terminal::Sent);
    assert_document_timeline_contract(&trace.sending, "sending", &trace.temporary_id);
    assert_document_timeline_contract(&trace.terminal, "sent", SERVER_ID);
}

#[test]
/// 证明 Go 业务拒绝不会误标 sent，并由已武装 timeout 收敛为 failed。
fn business_failure_waits_for_timeout_before_failed_frame() {
    let trace = G02ScenarioRunner::run_http(Terminal::BusinessFailed);
    assert_document_timeline_contract(&trace.sending, "sending", &trace.temporary_id);
    assert_document_timeline_contract(&trace.terminal, "failed", &trace.temporary_id);
}

#[test]
/// 证明 HTTP 基础设施错误经终态持久屏障后形成 failed Frame。
fn infrastructure_failure_projects_failed_document_card() {
    let trace = G02ScenarioRunner::run_http(Terminal::Failed);
    assert_document_timeline_contract(&trace.terminal, "failed", &trace.temporary_id);
}

#[test]
/// 证明 WS authority 可先行结算，迟到 HTTP 与重复 WS 均不重复终态写入。
fn ws_first_race_is_idempotent_and_updates_dialog_independently() {
    let trace = G02ScenarioRunner::run_ws_echo_with_dialog();
    assert_document_timeline_contract(&trace.timeline_sent, "sent", SERVER_ID);
    assert_dialog_contract_for(&trace.dialog, &g02_scenario_spec());
    assert_ne!(trace.timeline_corr, trace.dialog_corr);
}

#[test]
/// 证明冷启动只凭持久 row 即可恢复同一 typed Document view。
fn recovery_rebuilds_document_card_from_persisted_fact() {
    let trace = G02ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&trace.group_created, &g02_scenario_spec());
    assert_recovered_document_timeline_contract(&trace.recovered, SERVER_ID);
    let MessageContentVm::Document { summary, .. } = &first_message(&trace.recovered).content
    else {
        panic!("recovered G02 row must remain a document card")
    };
    assert_eq!(summary, DOCUMENT_URL);
}
