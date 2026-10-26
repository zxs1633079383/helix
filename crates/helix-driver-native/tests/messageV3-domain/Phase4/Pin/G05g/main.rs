use crate::shared;

/// MV3-G05g：真实发送 post 后执行取消置顶，并核对 Go WS 与 SQLite pinned=false。
#[tokio::test]
async fn mv3_g05g_real_chain() {
    shared::assert_post_pin_chain(
        "Phase4",
        "Pin",
        "MV3-G05g",
        "im_set_message_untop",
        "/api/cses/channel/remove/postPinned",
        false,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G05g FAIL: {error}"));
}
