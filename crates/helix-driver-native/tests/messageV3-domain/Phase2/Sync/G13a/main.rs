use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G13a：真实 native SQLite channel Scan → `im:read:result` 会话列表。
#[tokio::test]
async fn mv3_g13a_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let mut session = shared::start_real_helix_session(&context, &[])
        .await
        .unwrap_or_else(|error| panic!("MV3-G13a Helix session failed: {error}"));
    let channel_id = "chfixx0000000000000000003a";
    shared::insert_channel_fixture(
        &session.storage,
        channel_id,
        &context.actor.company_id,
        &context.actor.user_id,
        "P",
        "",
        &[context.actor.user_id.as_str()],
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G13a fixture failed: {error}"));
    let req_id = shared::run_token("g13a-dialog");
    session
        .command(
            "im_query_dialog_list",
            json!({"limit": 20, "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G13a command failed: {error}"));
    let event = session
        .wait_event(
            "im:read:result",
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G13a result failed: {error}"));
    let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G13a event JSON failed: {error}"));
    let items = event_json
        .pointer("/data/body/items")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("MV3-G13a result has no items: {event_json}"));
    assert!(items.iter().any(|item| item["id"] == channel_id));
    shared::write_gate_evidence(
        "Phase2",
        "Sync",
        "MV3-G13a",
        "PASS",
        json!({"command": "im_query_dialog_list", "storage": "SQLite channel Scan", "terminal_event": "im:read:result", "event": event_json}),
    )
    .unwrap_or_else(|error| panic!("MV3-G13a evidence failed: {error}"));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G13a cleanup failed: {error}"));
}
