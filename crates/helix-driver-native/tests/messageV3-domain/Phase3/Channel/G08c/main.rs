use crate::shared;
use serde_json::json;

/// MV3-G08c：真实 `channel/close` → local Persist → `im:channel:closed`。
#[tokio::test]
async fn mv3_g08c_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3",
        "Channel",
        "MV3-G08c",
        "im_channel_close",
        "/api/cses/channel/close",
        "im:channel:closed",
        |_, channel| json!({"channel_id": channel}),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G08c real chain failed: {error}"));
}
