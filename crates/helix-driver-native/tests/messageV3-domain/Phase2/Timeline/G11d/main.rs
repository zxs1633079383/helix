use crate::shared;
use serde_json::json;

/// MV3-G11d：真实 postId locate 经过 `/posts/postContext` authority 与 req_id terminal。
#[tokio::test]
async fn mv3_g11d_real_chain() {
    shared::assert_timeline_navigation_gate(
        "MV3-G11d",
        "im_locate_message",
        "/api/cses/posts/postContext",
        |channel, post_id, req_id| {
            json!({
                "channel_id": channel,
                "message_id": post_id,
                "page_size": 20,
                "req_id": req_id,
                "navigationToken": req_id,
            })
        },
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G11d FAIL: {error}"));
}
