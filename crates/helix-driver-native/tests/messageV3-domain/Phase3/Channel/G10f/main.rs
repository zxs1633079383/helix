use crate::shared;
use serde_json::json;

/// MV3-G10f：真实权限 partial patch，保留当前 Go 无 CAS 的真实合同事实。
#[tokio::test]
async fn mv3_g10f_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "Channel", "MV3-G10f", "im_channel_change_permission", "/api/cses/channel/change/permission", "im:channel:update",
        |_, channel| json!({"channel_id": channel, "mention_permission": "MEMBER", "req_id": shared::run_token("g10f")}),
    ).await.unwrap_or_else(|error| panic!("MV3-G10f real chain failed: {error}"));
}
