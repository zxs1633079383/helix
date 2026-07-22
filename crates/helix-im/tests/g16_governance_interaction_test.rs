use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use serde_json::{json, Value};

fn one_http(name: &str, payload: Value) -> (String, Value) {
    let effects = handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        Some("conn-g16"),
        Correlation::from_raw(16),
    )
    .expect("canonical governance command");
    match effects.as_slice() {
        [Effect::Http { req, .. }] => (
            req.url.clone(),
            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
        ),
        other => panic!("expected one HTTP effect, got {other:?}"),
    }
}

#[test]
fn five_governance_subflows_keep_distinct_wire_contracts() {
    let (close_url, close) = one_http("im_channel_close", json!({"channel_id":"c16"}));
    assert!(close_url.ends_with("/channel/close"));
    assert_eq!(close, json!({"channelId":"c16"}));

    let (leave_url, leave) = one_http("im_channel_leave", json!({"channel_id":"c16"}));
    assert!(leave_url.ends_with("/channel/member/leave"));
    assert_eq!(leave, json!({"channelId":"c16"}));

    let (members_url, members) = one_http(
        "im_channel_member_change",
        json!({"channel_id":"c16","team_id":"t1","self_id":"self","leave_user_ids":["u2"]}),
    );
    assert!(members_url.ends_with("/channel/member/change"));
    assert_eq!(members["leaveUsers"][0]["id"], "u2");

    let (role_url, role) = one_http(
        "im_channel_member_role",
        json!({"channel_id":"c16","user_ids":["u2"],"role":"ADMIN"}),
    );
    assert!(role_url.ends_with("/channel/member/change/role"));
    assert_eq!(role["role"], "ADMIN");

    let (settings_url, settings) = one_http(
        "im_channel_change_info",
        json!({"channel_id":"c16","purpose":"next","expected_version":7}),
    );
    assert!(settings_url.ends_with("/channel/change/info"));
    assert_eq!(settings["expectedVersion"], 7);
}

#[test]
fn member_role_rejects_unknown_domain_role_before_http() {
    let result = handle_outbound(
        "im_channel_member_role",
        &serde_json::to_vec(&json!({
            "channel_id":"c16",
            "user_ids":["u2"],
            "role":"SUPERUSER"
        }))
        .unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        None,
        Correlation::from_raw(160),
    );
    assert!(result.is_err());
}
