use crate::shared;

/// MV3-G05k：真实加载 bookmark 列表，并要求带回同一频道请求结果。
#[tokio::test]
async fn mv3_g05k_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G05k setup FAIL: {error}"));
    let req_id = shared::run_token("bookmark-load");
    let result = shared::assert_real_read_or_blocked(
        &context,
        "Phase4",
        "Bookmark",
        "MV3-G05k",
        &channel,
        "im_bookmark_load",
        "/api/cses/post/bookmark/load",
        serde_json::json!({"channel_id": channel, "page_number": 1, "page_size": 20, "req_id": req_id}),
        |envelope| {
            shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
        },
    )
    .await;
    let _ = shared::close_channel(&context, &channel).await;
    result.unwrap_or_else(|error| panic!("MV3-G05k FAIL: {error}"));
}
