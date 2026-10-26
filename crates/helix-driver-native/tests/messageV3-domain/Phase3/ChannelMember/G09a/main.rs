use crate::shared;
use serde_json::json;

/// MV3-G09a：真实成员移除 → Go WS `channel_member_update` → roster Persist/Emit。
#[tokio::test]
async fn mv3_g09a_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "ChannelMember", "MV3-G09a", "im_channel_member_change", "/api/cses/channel/member/change", "im:channel:member-updated",
        |context, channel| json!({"channel_id": channel, "team_id": context.actor.company_id, "self_id": context.actor.user_id, "join_user_ids": [], "leave_user_ids": [context.third.user_id]}),
    ).await.unwrap_or_else(|error| panic!("MV3-G09a real chain failed: {error}"));
}
