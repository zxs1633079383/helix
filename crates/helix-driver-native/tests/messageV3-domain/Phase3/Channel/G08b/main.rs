use crate::shared;
use serde_json::json;

/// MV3-G08b：真实 root post → `im_make_topic` → Go topic authority → channel-created projection。
#[tokio::test]
async fn mv3_g08b_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G08b channel setup failed: {error}"));
    let (_ack, frame) = shared::direct_post(
        &context,
        &channel,
        "TEXT",
        &shared::resource_name("topic-root"),
        json!({}),
        json!(["all"]),
        json!([]),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G08b root post failed: {error}"));
    let root_id = shared::extract_id(&frame, &["id", "postId", "post_id"])
        .unwrap_or_else(|| panic!("MV3-G08b root post has no id: {frame}"));
    let req_id = shared::run_token("g08b-topic");
    let result = shared::assert_real_mutation_or_blocked(
        &context,
        "Phase3",
        "Channel",
        "MV3-G08b",
        &channel,
        "im_make_topic",
        "/api/cses/posts/makeTopic",
        json!({"root_id": root_id, "user_ids": [context.observer.user_id], "display_name": shared::resource_name("topic"), "req_id": req_id}),
        "im:channel:created",
        |_| true,
    )
    .await;
    let _ = shared::close_channel(&context, &channel).await;
    result.unwrap_or_else(|error| panic!("MV3-G08b real chain failed: {error}"));
}
