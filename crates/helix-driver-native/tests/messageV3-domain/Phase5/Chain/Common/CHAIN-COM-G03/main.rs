use crate::shared;

/// CHAIN-COM-G03：真实并发追加、全局 seq 与 mutation 幂等 Gate。
#[tokio::test]
async fn chain_com_g03_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G03")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G03 FAIL: {error}"));
}
