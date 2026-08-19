use crate::shared;

/// CHAIN-COM-G02：真实草稿发布、revision 与 channel WS Gate。
#[tokio::test]
async fn chain_com_g02_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G02")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G02 FAIL: {error}"));
}
