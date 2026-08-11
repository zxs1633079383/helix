use crate::shared;
use serde_json::json;

/// MV3-G11c：真实 newer anchor 经过 `/posts/getPostsAfterIndex` 与 req_id terminal。
#[tokio::test]
async fn mv3_g11c_real_chain() {
    shared::assert_timeline_navigation_gate(
        "MV3-G11c",
        "im_load_newer_context",
        "/api/cses/posts/getPostsAfterIndex",
        |channel, post_id, req_id, create_at| {
            json!({
                "channel_id": channel,
                "anchor": {"postId": post_id, "createAt": create_at},
                "anchor_post_id": post_id,
                "page_size": 20,
                "req_id": req_id,
            })
        },
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G11c FAIL: {error}"));
}
