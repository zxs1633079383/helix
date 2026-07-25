use crate::scenario::{
    assert_timeline_contract, first_message, G01ScenarioRunner, Terminal, SERVER_ID,
};

#[test]
/// 证明 HTTP success 必须跨 terminal Persist/readback 后才发布 sent Frame。
fn http_success_releases_sent_frame_only_after_terminal_persist_readback() {
    let trace = G01ScenarioRunner::run_http(Terminal::Sent);
    assert_timeline_contract(&trace.sending, "sending", &trace.temporary_id);
    assert_timeline_contract(&trace.terminal, "sent", SERVER_ID);
    assert_eq!(first_message(&trace.terminal).id, SERVER_ID);
}

#[test]
/// 证明 HTTP error 必须跨 unsend Persist/readback 后才发布 failed Frame。
fn http_error_releases_failed_frame_only_after_unsend_readback() {
    let trace = G01ScenarioRunner::run_http(Terminal::Failed);
    assert_timeline_contract(&trace.sending, "sending", &trace.temporary_id);
    assert_timeline_contract(&trace.terminal, "failed", &trace.temporary_id);
    assert_eq!(first_message(&trace.terminal).id, trace.temporary_id);
}
