use crate::shared;
use serde_json::json;

/// MV3-G09b：真实成员角色变更 → Go WS roster update → Helix member projection。
#[tokio::test]
async fn mv3_g09b_real_chain() {
    shared::assert_new_channel_mutation(
        "Phase3", "ChannelMember", "MV3-G09b", "im_channel_member_role", "/api/cses/channel/member/change/role", "im:channel:member-updated",
        |context, channel| json!({"channel_id": channel, "user_ids": [context.observer.user_id], "role": "ADMIN"}),
    ).await.unwrap_or_else(|error| panic!("MV3-G09b real chain failed: {error}"));
}
