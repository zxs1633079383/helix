use crate::shared;

/// MV3-G04e：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g04e_real_chain() {
    shared::assert_schedule_chain("Phase1", "Schedule", "MV3-G04e", "immediate")
        .await
        .unwrap_or_else(|error| panic!("MV3-G04e FAIL: {error}"));
}
