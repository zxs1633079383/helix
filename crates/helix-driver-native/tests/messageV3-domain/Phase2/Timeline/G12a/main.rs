use crate::shared;

/// MV3-G12a：真实读取 Go `/posts/getReplies`，以 Helix `im:timeline:thread` 收口。
#[tokio::test]
async fn mv3_g12a_real_chain() {
    shared::assert_reply_read_or_blocked(
        "MV3-G12a",
        "im_get_replies",
        "/api/cses/posts/getReplies",
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G12a FAIL: {error}"));
}
