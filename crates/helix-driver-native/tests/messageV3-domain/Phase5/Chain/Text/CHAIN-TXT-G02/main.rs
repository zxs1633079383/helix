use crate::shared;

/// CHAIN-TXT-G02：真实双 Helix session/WS/SQLite 多设备同步 Gate。
#[tokio::test]
async fn chain_txt_g02_real_chain() {
    shared::run_text_chain_gate("CHAIN-TXT-G02")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-TXT-G02 FAIL: {error}"));
}
