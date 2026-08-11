use crate::shared;
use serde_json::json;
use tokio::time::Duration;

/// MV3-G09c：真实 nickname wire 已执行，但当前 Helix 事件名与 Gate 合同不一致，必须阻塞。
#[tokio::test]
async fn mv3_g09c_real_chain() {
    let (_guard, context) = shared::remote_setup().await;
    let channel = shared::create_channel(&context)
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c channel setup failed: {error}"));
    let mut session = shared::start_real_helix_session(&context, &[channel.as_str()])
        .await
        .unwrap_or_else(|error| panic!("MV3-G09c Helix session failed: {error}"));
    session.command("im_update_member_nickname", json!({"channel_id": channel, "user_id": context.observer.user_id, "nickname": shared::resource_name("nickname")})).await.unwrap_or_else(|error| panic!("MV3-G09c command failed: {error}"));
    match session
        .wait_event(
            "im:channel:memberNickname",
            |_| true,
            Duration::from_secs(25),
        )
        .await
    {
        Ok(event) => {
            let event_json: serde_json::Value = serde_json::from_slice(event.0.as_ref())
                .unwrap_or_else(|error| panic!("MV3-G09c event JSON failed: {error}"));
            shared::write_gate_evidence("Phase3", "ChannelMember", "MV3-G09c", "BLOCKED_PRECONDITION", json!({"reason": "producer emitted camelCase memberNickname while Gate contract requires im:channel:member-nickname and PersistOk ordering", "first_failing_layer": "Helix event contract", "event": event_json})).unwrap_or_else(|error| panic!("MV3-G09c evidence failed: {error}"));
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
