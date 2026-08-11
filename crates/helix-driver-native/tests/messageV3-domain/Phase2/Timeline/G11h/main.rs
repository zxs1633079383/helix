use crate::shared;
use serde_json::json;

/// MV3-G11h：只持有 postId 时走 initial `getPostsAfterIndex`，不伪造多余 postIds。
#[tokio::test]
async fn mv3_g11h_real_chain() {
    shared::assert_timeline_navigation_gate(
        "MV3-G11h",
        "im_get_posts_after_index",
        "/api/cses/posts/getPostsAfterIndex",
        |_, post_id, req_id| json!({"req_id": req_id, "post_id": post_id, "page_size": 20}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G11h FAIL: {error}"));
}
