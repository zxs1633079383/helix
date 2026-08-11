use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G06d：真实 `/post/read/list` 回执经过 Helix receipt projector 后只发 readers 事件。
#[tokio::test]
async fn mv3_g06d_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d setup channel failed: {error}"));
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d Helix session failed: {error}"));
    let (_response, post_frame) = shared::direct_post(
        &context,
        &channel,
        "TEXT",
        &shared::resource_name("readers-post"),
        json!({}),
        json!(["all"]),
        json!([]),
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G06d seed post failed: {error}"));
    let post_id = shared::extract_id(&post_frame, &["id", "postId", "post_id"])
        .unwrap_or_else(|| panic!("MV3-G06d post frame has no server id: {post_frame}"));
    let req_id = shared::run_token("g06d-readers");
    session
        .command(
            "im_post_read_list",
            json!({"post_ids": [post_id.clone()], "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d command failed: {error}"));
    let event = session
        .wait_event(
            "im:post:readers",
            |envelope| {
                envelope.get("data").is_some()
                    && serde_json::to_string(envelope)
                        .map(|encoded| encoded.contains(&post_id))
                        .unwrap_or(false)
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d readers event failed: {error}"));
    let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G06d event JSON failed: {error}"));
    assert!(
        event_json.get("event").and_then(serde_json::Value::as_str) == Some("im:post:readers"),
        "G06d must not downgrade to raw im:read:result"
    );
    shared::write_gate_evidence(
        "Phase2",
        "Read",
        "MV3-G06d",
        "PASS",
        json!({
            "command": "im_post_read_list",
            "go_http": "/api/cses/post/read/list",
            "terminal_event": "im:post:readers",
            "post_id": post_id,
            "event": event_json,
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G06d evidence failed: {error}"));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d Helix cleanup failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06d channel cleanup failed: {error}"));
}
