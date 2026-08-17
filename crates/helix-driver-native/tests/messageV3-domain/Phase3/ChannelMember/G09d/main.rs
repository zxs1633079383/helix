use crate::shared;
use serde_json::json;

/// MV3-G09d：真实单频道增量读回 → `im:read:result` 与 channel hydration。
#[tokio::test]
async fn mv3_g09d_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G09d channel setup failed: {error}"));
    let req_id = shared::run_token("g09d");
    shared::assert_real_read_or_blocked(
        &context,
        "Phase3",
        "ChannelMember",
        "MV3-G09d",
        &channel,
        "im_channel_load_increment_by_channel_id",
        "/api/cses/channel/load/incrementByChannelId",
        json!({"channel_id": channel, "req_id": req_id}),
        |envelope| shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G09d real chain failed: {error}"));
    let _ = shared::close_channel(&context, &channel).await;
}
