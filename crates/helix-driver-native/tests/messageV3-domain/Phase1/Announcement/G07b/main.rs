use crate::shared;

/// MV3-G07b：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g07b_real_chain() {
    shared::assert_announcement_save("Phase1", "Announcement", "MV3-G07b")
        .await
        .unwrap_or_else(|error| panic!("MV3-G07b FAIL: {error}"));
}
