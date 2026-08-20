use crate::shared;

/// CHAIN-COM-G06：真实已读、撤回、关闭、拒绝追加与 mutation 状态 Gate。
#[tokio::test]
async fn chain_com_g06_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G06")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G06 FAIL: {error}"));
}
