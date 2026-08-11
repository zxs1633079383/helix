use crate::shared;
use serde_json::json;

/// MV3-G08d：真实 `channel/member/leave` → Persist → closed terminalReason=left。
#[tokio::test]
async fn mv3_g08d_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3",
        "Channel",
        "MV3-G08d",
        "im_channel_leave",
        "/api/cses/channel/member/leave",
        "im:channel:closed",
        |_, channel| json!({"channel_id": channel}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G08d real chain failed: {error}"));
}
