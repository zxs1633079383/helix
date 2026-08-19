use crate::shared;

/// CHAIN-COM-G05：真实 HTTP authority → WS fan-out → Helix Emit → SQLite Gate。
#[tokio::test]
async fn chain_com_g05_real_chain() {
    shared::run_text_chain_gate("CHAIN-COM-G05")
        .await
        .unwrap_or_else(|error| panic!("CHAIN-COM-G05 FAIL: {error}"));
}
