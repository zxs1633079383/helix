use crate::shared;
use serde_json::json;

/// MV3-G10f：真实会话导向更新，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10f_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10f", "im_channel_change_orient", "/api/cses/channel/change/orient", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "orient": "GENERAL", "req_id": shared::run_token("g10f")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10f real chain failed: {error}"));
}
