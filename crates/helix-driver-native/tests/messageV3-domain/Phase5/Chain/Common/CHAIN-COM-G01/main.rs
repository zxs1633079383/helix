use crate::shared;

/// CHAIN-COM-G01：真实声明读取 Gate，从 Native Tick 走到 18066/SQLite。
#[tokio::test]
async fn chain_com_g01_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G01")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G01 FAIL: {error}"));
}
