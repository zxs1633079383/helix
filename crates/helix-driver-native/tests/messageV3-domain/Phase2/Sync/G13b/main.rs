use crate::shared;
use serde_json::json;

/// MV3-G13b：真实频道增量 hydration 读命令 → Go `/channel/load/incrementByChannelId`。
#[tokio::test]
async fn mv3_g13b_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G13b channel setup failed: {error}"));
    let req_id = shared::run_token("g13b-increment");
    shared::assert_real_read_or_blocked(
        &context,
        "Phase2",
        "Sync",
        "MV3-G13b",
        &channel,
        "im_channel_load_increment_by_channel_id",
        "/api/cses/channel/load/incrementByChannelId",
        json!({"channel_id": channel, "req_id": req_id}),
        |envelope| shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G13b real chain failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G13b cleanup failed: {error}"));
}
