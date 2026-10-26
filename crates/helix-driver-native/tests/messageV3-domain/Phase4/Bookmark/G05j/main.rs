use crate::shared;

/// MV3-G05j：真实创建后删除 bookmark，两个命令都走 Helix→Go read relay。
#[tokio::test]
async fn mv3_g05j_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G05j setup FAIL: {error}"));
    let result = async {
        let (_ack, frame) = shared::direct_post(
            &context,
            &channel,
            "TEXT",
            &shared::resource_name("bookmark-delete-seed"),
            serde_json::json!({}),
            serde_json::json!(["all"]),
            serde_json::json!([]),
        )
        .await?;
        let post_id = shared::extract_id(&frame, &["id", "postId", "post_id"])
            .ok_or_else(|| "MV3-G05j post WS frame has no post id".to_string())?;
        let create_req = shared::run_token("bookmark-create");
        shared::assert_real_read_or_blocked(
            &context,
            "Phase4",
            "Bookmark",
            "MV3-G05j",
            &channel,
            "im_bookmark_create",
            "/api/cses/post/bookmark/create",
            serde_json::json!({"channel_id": channel, "post_ids": [post_id.clone()], "req_id": create_req}),
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(create_req.as_str())
            },
        )
        .await?;
        let delete_req = shared::run_token("bookmark-delete");
        shared::assert_real_read_or_blocked(
            &context,
            "Phase4",
            "Bookmark",
            "MV3-G05j",
            &channel,
            "im_bookmark_delete",
            "/api/cses/post/bookmark/delete",
            serde_json::json!({"channel_id": channel, "post_id": post_id, "req_id": delete_req}),
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(delete_req.as_str())
            },
        )
        .await
    }
    .await;
    let _ = shared::close_channel(&context, &channel).await;
    result.unwrap_or_else(|error| panic!("MV3-G05j FAIL: {error}"));
}
