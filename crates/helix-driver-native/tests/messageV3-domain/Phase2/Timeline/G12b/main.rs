use crate::shared;

/// MV3-G12b：真实读取 Go `/posts/getReplyBranch`，以 Helix thread append 收口。
#[tokio::test]
async fn mv3_g12b_real_chain() {
    shared::assert_reply_read_or_blocked(
        "MV3-G12b",
        "im_get_reply_branch",
        "/api/cses/posts/getReplyBranch",
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G12b FAIL: {error}"));
}
