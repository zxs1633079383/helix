use crate::shared;

/// MV3-G08a：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g08a_real_chain() {
    shared::assert_channel_create_chain("Phase1", "Channel", "MV3-G08a")
        .await
        .unwrap_or_else(|error| panic!("MV3-G08a real chain failed: {error}"));
}
