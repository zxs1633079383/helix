use crate::shared;
use serde_json::json;

/// MV3-G09e：真实在线状态读取，保留 Go 当前 POST wire 合同而非 prompt 的 GET 猜测。
#[tokio::test]
async fn mv3_g09e_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G09e channel setup failed: {error}"));
    let req_id = shared::run_token("g09e");
    shared::assert_real_read_or_blocked(
        &context,
        "Phase3",
        "ChannelMember",
        "MV3-G09e",
        &channel,
        "im_channel_online_status",
        "/api/cses/channel/onlineStatus",
        json!({"channel_ids": [channel], "req_id": req_id}),
        |envelope| shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G09e real chain failed: {error}"));
    let _ = shared::close_channel(&context, &channel).await;
}
