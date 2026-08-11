use crate::shared;
use serde_json::json;

/// MV3-G05c：独立 Gate 真实链测试；要求业务 SUCCESS、WS/Projection 与最终状态证据。
#[tokio::test]
async fn mv3_g05c_real_chain() {
    shared::assert_post_action(
        "Phase1",
        "Post",
        "MV3-G05c",
        "posts/urgentPost",
        json!({"channelId": ""}),
        "post_update",
        Some("urgent"),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G05c FAIL: {error}"));
}
