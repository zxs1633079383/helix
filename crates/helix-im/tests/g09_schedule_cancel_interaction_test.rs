//! G09 focused contract: canceled authority clears draft content; delivery remains G01-owned.

use helix_core::Effect;
use helix_im::acl::to_effect_s1::emit_schedule_canceled;
use helix_im::state::ChannelId;
use serde_json::json;

#[test]
fn cancel_terminal_projection_cannot_resurrect_retired_schedule_body() {
    let channel_id = ChannelId::from_str("chfixx00000000000000000009").expect("valid test id");
    let Effect::Emit { event } = emit_schedule_canceled(
        channel_id,
        &json!({
            "channelId": "chfixx00000000000000000009",
            "message": "must disappear",
            "messagePreview": "must disappear",
            "revision": 9
        }),
        None,
        true,
    ) else {
        panic!("schedule cancel must emit terminal projection")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(payload["data"]["scheduleStatus"], "canceled");
    assert_eq!(payload["data"]["hasSchedulePost"], false);
    assert!(payload["data"]["scheduleMessage"].is_null());
    assert!(payload["data"]["scheduleMessagePreview"].is_null());
    assert!(payload["data"].get("deliveredMessage").is_none());
}
