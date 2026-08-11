use crate::shared;

/// MV3-G02c：执行默认测试并记录 durable failed-row 前置仍未具备的事实。
#[tokio::test]
async fn mv3_g02c_real_chain() {
    shared::record_unverified("Phase1", "Post", "MV3-G02c", "retry needs a durable failed row and an approved real retry fixture; no synthetic retry is allowed.").unwrap();
}
