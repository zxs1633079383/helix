use crate::shared;
use serde_json::json;

/// MV3-G11b：真实 older anchor 经过 `/posts/getPostsAfterIndex` 与 req_id terminal。
#[tokio::test]
async fn mv3_g11b_real_chain() {
    shared::assert_timeline_navigation_gate(
        "MV3-G11b",
        "im_load_older_context",
        "/api/cses/posts/getPostsAfterIndex",
        |channel, post_id, req_id| {
            json!({
                "channel_id": channel,
                "anchor": {"postId": post_id, "createAt": 0},
                "anchor_post_id": post_id,
                "page_size": 20,
                "req_id": req_id,
            })
        },
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G11b FAIL: {error}"));
}
