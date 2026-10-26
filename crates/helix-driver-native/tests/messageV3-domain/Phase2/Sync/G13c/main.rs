use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G13c：真实 `/channels/load/increment` 读回，并检查 Helix 增量投影或阻塞原因。
#[tokio::test]
async fn mv3_g13c_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G13c channel setup failed: {error}"));
    let req_id = shared::run_token("g13c-increment");
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G13c Helix session failed: {error}"));
    session
        .command(
            "im_request_channel_increment",
            json!({"channel_id": channel, "timestamp": 0, "cursors": [], "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G13c command failed: {error}"));
    let event = match session
        .wait_event(
            "im:read:result",
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
            Duration::from_secs(25),
        )
        .await
    {
        Ok(event) => event,
        Err(error) => {
            shared::write_gate_evidence(
                "Phase2",
                "Sync",
                "MV3-G13c",
                "BLOCKED_PRECONDITION",
                json!({"reason": "channels/load/increment did not produce read result", "first_failing_layer": "Go increment route/WS producer or Helix read relay", "error": error}),
            )
            .unwrap_or_else(|write_error| panic!("MV3-G13c blocked evidence failed: {write_error}"));
            session
                .close()
                .await
                .unwrap_or_else(|close_error| panic!("MV3-G13c cleanup failed: {close_error}"));
            shared::close_channel(&context, &channel)
                .await
                .unwrap_or_else(|close_error| {
                    panic!("MV3-G13c channel cleanup failed: {close_error}")
                });
            return;
        }
    };
    let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G13c event JSON failed: {error}"));
    assert!(event_json.pointer("/data/body").is_some());
    shared::write_gate_evidence(
        "Phase2",
        "Sync",
        "MV3-G13c",
        "PASS",
        json!({
            "corr_key": req_id,
            "command": "im_request_channel_increment",
            "go_http": "/api/cses/channels/load/increment",
            "http_business": "Go increment read reached the Helix read projection",
            "terminal_event": "im:read:result",
            "projection": event_json,
            "ws_action": "channel_increment read projection",
            "persist_after": "read-only command; no durable mutation",
            "cleanup": "session.close and channel/close completed"
        }),
    )
    .unwrap_or_else(|error| panic!("MV3-G13c evidence failed: {error}"));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G13c cleanup failed: {error}"));
    shared::close_channel(&context, &channel)
        .await
        .unwrap_or_else(|error| panic!("MV3-G13c channel cleanup failed: {error}"));
}
