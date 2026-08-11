use crate::shared;

/// MV3-G01k：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g01k_real_chain() {
    shared::assert_reply_chain("Phase1", "Post", "MV3-G01k")
        .await
        .unwrap_or_else(|error| panic!("MV3-G01k FAIL: {error}"));
}
