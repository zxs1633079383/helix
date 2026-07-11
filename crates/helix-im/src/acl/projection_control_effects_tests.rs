use super::emit_post_batch_updated;
use crate::state::ChannelId;
use helix_core::Effect;
use serde_json::json;

#[test]
fn post_batch_updated_emits_render_ready_messages_for_thin_clients() {
    let channel_id = "channel1234567890abcdefghi";
    let effect = emit_post_batch_updated(
        ChannelId::from_str(channel_id).expect("channel id"),
        &json!([{
            "id": "post-1",
            "temporaryId": "tmp-1",
            "channelId": channel_id,
            "userId": "678",
            "type": "TEXT",
            "message": "撤回前内容",
            "readBits": "10",
            "revoke": true,
            "createAt": 42
        }]),
    );

    let Effect::Emit { event } = effect else {
        panic!("expected emit effect");
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).expect("json");
    let post = &payload["data"]["posts"][0];
    assert_eq!(payload["event"], "im:post:batch-updated");
    assert_eq!(post["id"], "post-1");
    assert_eq!(post["channelId"], channel_id);
    assert_eq!(post["text"], "撤回前内容");
    assert_eq!(post["sendStatus"], "sent");
    assert_eq!(post["revoke"], true);
    assert_eq!(post["revoked"], true);
}
