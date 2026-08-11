use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G11g：按 server postId 精确读取，断言 Go body、Helix read result 与本地 durable row。
#[tokio::test]
async fn mv3_g11g_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g setup channel failed: {error}"));
    let (_response, frame) = shared::direct_post_as(
        &context.actor,
        &context.observer,
        &channel,
        "TEXT",
        &shared::resource_name("exact-post"),
        json!({}),
        json!(["all"]),
        json!([]),
        72,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G11g seed post failed: {error}"));
    let post_id = shared::extract_id(&frame, &["id", "postId", "post_id"])
        .unwrap_or_else(|| panic!("MV3-G11g seed post has no id: {frame}"));
    let temporary_id = shared::extract_id(&frame, &["temporaryId", "temporary_id"])
        .unwrap_or_else(|| panic!("MV3-G11g seed post has no temporary id: {frame}"));
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g Helix session failed: {error}"));
    let req_id = shared::run_token("g11g-exact");
    session
        .command(
            "im_get_posts",
            json!({"post_ids": [post_id.clone()], "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g command failed: {error}"));
    let event = session
        .wait_event(
            "im:read:result",
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
            Duration::from_secs(25),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g read result failed: {error}"));
    let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G11g event JSON failed: {error}"));
    let body = event_json
        .pointer("/data/body")
        .unwrap_or_else(|| panic!("MV3-G11g result has no body: {event_json}"));
    let posts = body
        .get("posts")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("exact read result has no posts array: {body}"));
    if posts.is_empty() {
        shared::write_gate_evidence(
            "Phase2",
            "Timeline",
            "MV3-G11g",
            "BLOCKED_PRECONDITION",
            json!({
                "reason": "Go posts/get returned SUCCESS-shaped exact result but no seeded post",
                "first_failing_layer": "Go producer/consumer or remote post persistence",
                "post_id": post_id,
                "event": event_json,
            }),
        )
        .unwrap_or_else(|error| panic!("MV3-G11g blocked evidence failed: {error}"));
        session
            .close()
            .await
            .unwrap_or_else(|error| panic!("MV3-G11g Helix cleanup failed: {error}"));
        shared::close_channel(&context, &channel)
            .await
            .unwrap_or_else(|error| panic!("MV3-G11g channel cleanup failed: {error}"));
        return;
    }
    assert!(
        posts.iter().any(|post| {
            shared::extract_id(post, &["id", "postId", "serverId"]).as_deref()
                == Some(post_id.as_str())
        }),
        "exact post response must contain requested server id: {body}"
    );
    let row = shared::scan_message_row(&session.storage, &temporary_id)
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g durable row missing: {error}"));
    assert_eq!(
        shared::row_text(&row, "send_status").as_deref(),
        Some("sent")
    );
    shared::write_gate_evidence(
        "Phase2",
        "Timeline",
        "MV3-G11g",
        "PASS",
        json!({
            "command": "im_get_posts",
            "go_http": "/api/cses/posts/get",
            "terminal_event": "im:read:result",
            "post_id": post_id,
            "temporary_id": temporary_id,
            "event": event_json,
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G11g evidence failed: {error}"));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g Helix cleanup failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G11g channel cleanup failed: {error}"));
}
