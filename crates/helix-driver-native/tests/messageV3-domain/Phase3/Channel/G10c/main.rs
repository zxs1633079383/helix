use crate::shared;
use serde_json::json;

/// MV3-G10c：真实群简介更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10c_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10c", "im_channel_change_info", "/api/cses/channel/change/info", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "purpose": shared::resource_name("purpose"), "req_id": shared::run_token("g10c")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10c real chain failed: {error}"));
}
