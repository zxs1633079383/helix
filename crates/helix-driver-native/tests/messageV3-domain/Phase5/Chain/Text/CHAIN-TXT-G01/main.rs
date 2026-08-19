use crate::shared;

/// CHAIN-TXT-G01：真实 TEXT 追加、无 categoryIds 和首尾折叠投影 Gate。
#[tokio::test]
async fn chain_txt_g01_real_chain() {
    shared::run_text_chain_gate("CHAIN-TXT-G01")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-TXT-G01 FAIL: {error}"));
}
