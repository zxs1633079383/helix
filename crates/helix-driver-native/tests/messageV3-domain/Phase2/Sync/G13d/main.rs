use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G13d：真实 native SQLite channel Scan → parent-scoped topic result。
#[tokio::test]
async fn mv3_g13d_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let mut session = shared::start_real_helix_session(&context, &[])
        .await
        .unwrap_or_else(|error| panic!("MV3-G13d Helix session failed: {error}"));
    let parent = "chfixx00000000000000000064";
    let topic = "chfixx00000000000000000065";
    shared::insert_channel_fixture(
        &session.storage,
        parent,
        &context.actor.company_id,
        &context.actor.user_id,
        "P",
        "",
        &[context.actor.user_id.as_str()],
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G13d parent fixture failed: {error}"));
    shared::insert_channel_fixture(
        &session.storage,
        topic,
        &context.actor.company_id,
        &context.actor.user_id,
        "T",
        parent,
        &[context.actor.user_id.as_str()],
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G13d topic fixture failed: {error}"));
    let req_id = shared::run_token("g13d-subtopics");
    session
        .command(
            "im_query_subtopics",
            json!({"parentChannelId": parent, "req_id": req_id}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G13d command failed: {error}"));
    let event = session
        .wait_event(
            "im:read:result",
            |envelope| {
                shared::data_string(envelope, &["req_id"]).as_deref() == Some(req_id.as_str())
            },
            Duration::from_secs(20),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G13d result failed: {error}"));
    let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
        .unwrap_or_else(|error| panic!("MV3-G13d event JSON failed: {error}"));
    let items = event_json
        .pointer("/data/body/items")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("MV3-G13d result has no items: {event_json}"));
    assert!(items.iter().any(|item| item["id"] == topic));
    shared::write_gate_evidence(
        "Phase2",
        "Sync",
        "MV3-G13d",
        "PASS",
        json!({"command": "im_query_subtopics", "storage": "SQLite channel Scan", "parent": parent, "topic": topic, "terminal_event": "im:read:result", "event": event_json}),
    )
    .unwrap_or_else(|error| panic!("MV3-G13d evidence failed: {error}"));
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G13d cleanup failed: {error}"));
}
