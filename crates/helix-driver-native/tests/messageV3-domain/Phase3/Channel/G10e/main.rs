use crate::shared;
use serde_json::json;

/// MV3-G10e：真实 purpose mutation，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10e_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10e", "im_channel_change_purpose", "/api/cses/channel/change/purpose", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "purpose": shared::resource_name("purpose"), "req_id": shared::run_token("g10e")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10e real chain failed: {error}"));
}
