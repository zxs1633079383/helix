use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G09c：真实 nickname wire、PersistFire 行和 canonical member-nickname emit 全链断言。
#[tokio::test]
async fn mv3_g09c_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c channel setup failed: {error}"));
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c Helix session failed: {error}"));
    shared::insert_channel_member_fixture(
        &session.storage,
        &channel,
        &context.actor.user_id,
        &context.actor.company_id,
    )
    .await
    .unwrap_or_else(|error| panic!("MV3-G09c fixture failed: {error}"));
    let nick_name = shared::resource_name("nickname");
    session
        .command(
            "im_update_member_nickname",
            json!({"channel_id": channel, "user_id": context.actor.user_id, "nickname": nick_name}),
        )
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c command failed: {error}"));
    match session
        .wait_event(
            "im:channel:member-nickname",
            |envelope| {
                shared::data_string(envelope, &["channelId", "channel_id"]).as_deref()
                    == Some(channel.as_str())
                    && shared::data_string(envelope, &["userId", "user_id"]).as_deref()
                        == Some(context.actor.user_id.as_str())
                    && shared::data_string(envelope, &["nickName", "nick_name"]).as_deref()
                        == Some(nick_name.as_str())
            },
            Duration::from_secs(25),
        )
        .await
    {
        Ok(event) => {
            let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
                .unwrap_or_else(|error| panic!("MV3-G09c event JSON failed: {error}"));
            let row = shared::wait_channel_member_nickname(
                &session.storage,
                &channel,
                &context.actor.user_id,
                &nick_name,
                Duration::from_secs(8),
            )
            .await
            .unwrap_or_else(|error| panic!("MV3-G09c PersistOk failed: {error}"));
            shared::write_gate_evidence("Phase3", "ChannelMember", "MV3-G09c", "PASS", json!({"command": "im_update_member_nickname", "go_http": "/api/cses/channel/member/change/nickname", "terminal_event": "im:channel:member-nickname", "event": event_json, "persist_after": {"table": "channel_member", "channel_id": channel, "user_id": context.actor.user_id, "nick_name": shared::row_text(&row, "nick_name")}})).unwrap_or_else(|error| panic!("MV3-G09c evidence failed: {error}"));
        }
        Err(error) => {
            shared::write_gate_evidence("Phase3", "ChannelMember", "MV3-G09c", "BLOCKED_PRECONDITION", json!({"reason": "nickname terminal event not observed", "first_failing_layer": "Go route/WS or Helix nickname handler", "error": error})).unwrap_or_else(|write_error| panic!("MV3-G09c blocked evidence failed: {write_error}"));
        }
    }
    session
        .close()
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c cleanup failed: {error}"));
    let _ = shared::close_channel(&context, &channel).await;
}
