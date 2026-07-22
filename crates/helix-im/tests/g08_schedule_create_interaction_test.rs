//! G08 focused contract: schedule hydrate remains owner/channel scoped.

use helix_im::acl::to_effect_s1::owner_schedule_hydration;
use serde_json::json;

#[test]
fn get_schedule_hydration_rejects_observer_and_cross_channel_content() {
    let channel_id = "chfixx00000000000000000008";
    let authority = json!({
        "channelId": channel_id,
        "scheduleId": "schedule-8",
        "scheduleOwnerUserId": "444",
        "message": "owner-only content",
        "sendAt": 1_780_000_000_000_i64,
        "revision": 8
    });

    let owner = owner_schedule_hydration(&authority, channel_id, "444")
        .expect("matching owner and channel hydrate");
    assert_eq!(owner["scheduleMessage"], "owner-only content");
    assert!(owner_schedule_hydration(&authority, channel_id, "678").is_none());
    assert!(owner_schedule_hydration(&authority, "channel-other", "444").is_none());
}
