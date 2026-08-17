use crate::shared;

/// MV3-G05i：真实创建 post bookmark，并要求 Helix read relay 收到业务结果。
#[tokio::test]
async fn mv3_g05i_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G05i setup FAIL: {error}"));
    let result = async {
        let (_ack, frame) = shared::direct_post(
            &context,
            &channel,
            "TEXT",
            &shared::resource_name("bookmark-seed"),
            serde_json::json!({}),
            serde_json::json!(["all"]),
            serde_json::json!([]),
        )
        .await?;
        let post_id = shared::extract_id(&frame, &["id", "postId", "post_id"])
            .ok_or_else(|| "MV3-G05i post WS frame has no post id".to_string())?;
        let req_id = shared::run_token("bookmark-create");
        shared::assert_real_read_or_blocked(
            &context,
            "Phase4",
            "Bookmark",
            "MV3-G05i",
            &channel,
            "im_bookmark_create",
            "/api/cses/post/bookmark/create",
            serde_json::json!({"channel_id": channel, "post_ids": [post_id], "req_id": req_id}),
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
        )
        .await
    }
    .await;
    let _ = shared::close_channel(&context, &channel).await;
    result.unwrap_or_else(|error| panic!("MV3-G05i FAIL: {error}"));
}
