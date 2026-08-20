use crate::shared;

/// CHAIN-COM-G04：真实 get、limit/middleCursor、TextWindow 与 SQLite Gate。
#[tokio::test]
async fn chain_com_g04_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G04")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G04 FAIL: {error}"));
}
