use crate::shared;
use serde_json::json;

/// MV3-G10a：真实频道聚合设置更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10a_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10a", "im_channel_change_info", "/api/cses/channel/change/info", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "display_name": shared::resource_name("display"), "req_id": shared::run_token("g10a")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10a real chain failed: {error}"));
}
