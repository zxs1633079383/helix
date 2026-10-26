use crate::shared;

/// MV3-G05e：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g05e_real_chain() {
    shared::assert_template_receipts("Phase1", "Post", "MV3-G05e")
        .await
        .unwrap_or_else(|error| panic!("MV3-G05e FAIL: {error}"));
}
