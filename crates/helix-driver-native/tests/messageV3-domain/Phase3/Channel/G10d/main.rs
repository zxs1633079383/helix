use crate::shared;
use serde_json::json;

/// MV3-G10d：真实 orient mutation，核对 Go authority、PersistOk 与 channel update。
#[tokio::test]
async fn mv3_g10d_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10d", "im_channel_change_orient", "/api/cses/channel/change/orient", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "orient": "GENERAL", "req_id": shared::run_token("g10d")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10d real chain failed: {error}"));
}
