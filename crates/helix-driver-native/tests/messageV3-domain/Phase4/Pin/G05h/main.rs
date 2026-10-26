use crate::shared;

/// MV3-G05h：真实读取频道置顶列表，并要求 read result 带回同一请求上下文。
#[tokio::test]
async fn mv3_g05h_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G05h setup FAIL: {error}"));
    let req_id = shared::run_token("pin-load");
    let result = shared::assert_real_read_or_blocked(
        &context,
        "Phase4",
        "Pin",
        "MV3-G05h",
        &channel,
        "im_channel_load_post_pinned",
        "/api/cses/channel/load/postPinned",
        serde_json::json!({"channel_id": channel, "req_id": req_id}),
        |envelope| shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str()),
    )
    .await;
    let _ = shared::close_channel(&context, &channel).await;
    result.unwrap_or_else(|error| panic!("MV3-G05h FAIL: {error}"));
}
