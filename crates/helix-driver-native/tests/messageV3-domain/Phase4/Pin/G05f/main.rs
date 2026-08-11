use crate::shared;

/// MV3-G05f：真实发送 post 后执行置顶，并核对 Go WS 与 SQLite pinned=true。
#[tokio::test]
async fn mv3_g05f_real_chain() {
    shared::assert_post_pin_chain(
        "Phase4",
        "Pin",
        "MV3-G05f",
        "im_set_message_top",
        "/api/cses/channel/add/postPinned",
        true,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G05f FAIL: {error}"));
}
