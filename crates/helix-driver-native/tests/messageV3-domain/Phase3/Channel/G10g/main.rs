use crate::shared;
use serde_json::json;

/// MV3-G10g：真实频道权限更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10g_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10g", "im_channel_change_permission", "/api/cses/channel/change/permission", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "mention_permission": "MEMBER", "req_id": shared::run_token("g10g")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10g real chain failed: {error}"));
}
