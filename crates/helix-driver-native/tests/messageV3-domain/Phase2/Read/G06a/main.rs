use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G06a：频道已读通过 Helix command、Go `/channels/view` 与 req_id read result 完成。
#[tokio::test]
async fn mv3_g06a_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a setup channel failed: {error}"));
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a Helix session failed: {error}"));
    let req_id = shared::run_token("g06a-read");
    session
        .command(
            "im_channels_view",
            json!({"channels": [{"id": channel, "isRoot": true}], "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a command failed: {error}"));
    let event = session
        .wait_event(
            "im:read:result",
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a read result failed: {error}"));
    let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G06a event JSON failed: {error}"));
    let body = envelope
        .pointer("/data/body")
        .unwrap_or_else(|| panic!("MV3-G06a result has no Go body: {envelope}"));
    assert_eq!(
        body.get("status").and_then(serde_json::Value::as_str),
        Some("SUCCESS"),
        "MV3-G06a must assert Go business SUCCESS, not HTTP status only"
    );
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a Helix cleanup failed: {error}"));
    shared::write_gate_evidence(
        "Phase2",
        "Read",
        "MV3-G06a",
        "PASS",
        json!({
            "command": "im_channels_view",
            "go_http": "/api/cses/channels/view",
            "http_business": "Go body status=SUCCESS",
            "projection": "im:read:result",
            "persist_after": "terminal read result correlated by req_id",
            "cleanup": {"status": "PASS", "transport": "closed", "remote_channel": channel},
            "corr_key": req_id,
            "terminal_event": "im:read:result",
            "req_id": req_id,
            "expected_ws": "none",
            "body": body,
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G06a evidence failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G06a channel cleanup failed: {error}"));
}
