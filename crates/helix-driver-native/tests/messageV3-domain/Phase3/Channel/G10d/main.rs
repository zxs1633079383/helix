use crate::shared;
use serde_json::json;

/// MV3-G10d：真实会话置顶更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10d_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10d", "im_channel_change_top", "/api/cses/channel/change/top", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "top": true, "req_id": shared::run_token("g10d")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10d real chain failed: {error}"));
}
