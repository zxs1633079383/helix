use crate::shared;
use serde_json::json;

/// MV3-G10b：真实公告设置更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10b_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10b", "im_channel_change_notice", "/api/cses/channel/change/notice", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "notice": {"text": shared::resource_name("notice")}, "req_id": shared::run_token("g10b")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10b real chain failed: {error}"));
}
