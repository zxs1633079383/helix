use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G06b：双 persona 真实发送 post 后，经 Helix `im_post_read`、Go HTTP/WS、SQLite
/// exact-post read-back 与作者侧 `im:post:read` 终态形成一条可审计链。
#[tokio::test]
async fn mv3_g06b_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b setup channel failed: {error}"));
    let mut author =
        shared::start_real_helix_session_as(&context, &context.observer, &[channel.as_str()])
            .await
            .unwrap_or_else(|error| panic!("MV3-G06b author Helix session failed: {error}"));
    let (_post_response, post_frame) = shared::direct_post_as(
        &context.observer,
        &context.actor,
        &channel,
        "TEXT",
        &shared::resource_name("read-post"),
        json!({}),
        json!(["all"]),
        json!([]),
        61,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G06b seed post failed: {error}"));
    let temporary_id = shared::extract_id(&post_frame, &["temporaryId", "temporary_id"])
        .unwrap_or_else(|| panic!("MV3-G06b post frame has no temporaryId: {post_frame}"));
    let post_id = shared::extract_id(&post_frame, &["id", "postId", "post_id"])
        .unwrap_or_else(|| panic!("MV3-G06b post frame has no server id: {post_frame}"));
    let reader = shared::start_real_helix_session_as(&context, &context.actor, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b reader Helix session failed: {error}"));
    let received = author
        .wait_event(
            "im:post:received",
            |envelope| {
                shared::data_string(envelope, &["temporaryId", "temporary_id"]).as_deref()
                    == Some(temporary_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b seed post did not project: {error}"));
    let seeded_row = shared::scan_message_row(&author.storage, &temporary_id)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b seed post missing from SQLite: {error}"));
    assert_eq!(
        shared::row_text(&seeded_row, "send_status").as_deref(),
        Some("sent"),
        "seed post must be durable before read command"
    );
    // post_read 只发给消息作者；author session 消费 Go WS，reader session 只负责发起 HTTP。
    reader
        .command(
            "im_post_read",
            json!({"channel_id": channel, "posts": [post_id.clone()]}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b command failed: {error}"));
    let read_event = author
        .wait_event(
            "im:post:read",
            |envelope| {
                shared::data_string(envelope, &["postId", "post_id"]).as_deref()
                    == Some(post_id.as_str())
                    && shared::data_string(envelope, &["readerUserId", "reader_user_id"]).as_deref()
                        == Some(context.actor.user_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b Helix read event failed: {error}"));
    let row_after = shared::scan_message_row(&author.storage, &temporary_id)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b read-back failed: {error}"));
    assert!(
        shared::row_text(&row_after, "read_bits").is_some(),
        "read terminal must leave a read_bits column read-back"
    );
    shared::write_gate_evidence(
        "Phase2",
        "Read",
        "MV3-G06b",
        "PASS",
        json!({
            "command": "im_post_read",
            "go_http": "/api/cses/post/read",
            "go_ws": "post_read (consumed by author NativeTransport)",
            "helix_event": "im:post:read",
            "persist": ["message.send_status=sent", "message.read_bits read-back"],
            "temporary_id": temporary_id,
            "post_id": post_id,
            "seed_received": serde_json::from_slice::<serde_json::Value>(received.0.as_ref()).unwrap_or_default(),
            "seed_row_send_status": shared::row_text(&seeded_row, "send_status"),
            "read_event": serde_json::from_slice::<serde_json::Value>(read_event.0.as_ref()).unwrap_or_default(),
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G06b evidence failed: {error}"));
    author
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b author cleanup failed: {error}"));
    reader
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b reader cleanup failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06b channel cleanup failed: {error}"));
}
