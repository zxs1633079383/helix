use crate::scenario::{
    assert_dialog_contract, assert_timeline_contract, G01ScenarioRunner, SERVER_ID,
};

#[test]
/// 证明 WS echo 的 timeline 结算与 dialog patch 使用独立 correlation。
fn ws_post_inbound_reconciles_timeline_and_independently_patches_dialog() {
    let trace = G01ScenarioRunner::run_ws_echo_with_dialog();
    assert_ne!(trace.timeline_corr, trace.dialog_corr);
    assert_timeline_contract(&trace.sending, "sending", &trace.temporary_id);
    assert_timeline_contract(&trace.timeline_sent, "sent", SERVER_ID);
    assert_dialog_contract(&trace.dialog);
}
