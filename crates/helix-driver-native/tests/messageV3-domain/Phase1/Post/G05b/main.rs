use crate::shared;
use serde_json::json;

/// MV3-G05b：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g05b_real_chain() {
    shared::assert_post_action(
        "Phase1",
        "Post",
        "MV3-G05b",
        "posts/quickReply",
        json!({"emoji": "👍"}),
        "post_update",
        Some("quickReply"),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G05b FAIL: {error}"));
}
