//! channel 写族补全命令测试：endpoint + camelCase body + 键名陷阱（id vs channelId）+ 必填零信任。

use helix_core::effect::Effect;
use helix_core::Correlation;
use serde_json::json;

use crate::outbound::registry::{handle_outbound, is_outbound};

/// 调度一条命令，断言单 Http effect 并返回 (url, body)。
fn dispatch(name: &str, args: serde_json::Value) -> (String, serde_json::Value) {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&args).unwrap();
    let effects = handle_outbound(name, &payload, "http://h/api", "http://h", Some("c1"), corr)
        .unwrap_or_else(|e| panic!("{name} should dispatch: {e:?}"));
    assert_eq!(effects.len(), 1, "{name} 应单条 Http effect");
    match &effects[0] {
        Effect::Http { req, .. } => {
            let body: serde_json::Value =
                serde_json::from_slice(req.body.as_ref().expect("body")).unwrap();
            (req.url.clone(), body)
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

#[test]
fn display_name_endpoint_and_id_key() {
    assert!(is_outbound("im_channel_change_display_name"));
    let (url, body) = dispatch(
        "im_channel_change_display_name",
        json!({ "channel_id": "c1", "display_name": "新群名" }),
    );
    assert!(url.ends_with("/channel/change/displayName"), "url={url}");
    // 键名陷阱：用 id（非 channelId）作 channelId 键。
    assert_eq!(body["id"], "c1");
    assert_eq!(body["displayName"], "新群名");
    assert!(body.get("channelId").is_none());
}

#[test]
fn orient_and_purpose_use_id_key() {
    let (url, body) = dispatch(
        "im_channel_change_orient",
        json!({ "channel_id": "c1", "orient": "left" }),
    );
    assert!(url.ends_with("/channel/change/orient"), "url={url}");
    assert_eq!(body["id"], "c1");
    assert_eq!(body["orient"], "left");

    let (url2, body2) = dispatch(
        "im_channel_change_purpose",
        json!({ "channel_id": "c1", "purpose": "团队协作群" }),
    );
    assert!(url2.ends_with("/channel/change/purpose"), "url={url2}");
    assert_eq!(body2["id"], "c1");
    assert_eq!(body2["purpose"], "团队协作群");
}

#[test]
fn add_and_remove_manger_channel_id_key_and_users() {
    let users = json!([{ "id": "u1", "name": "张三", "role": "admin", "teamId": "t1" }]);
    let (url, body) = dispatch(
        "im_channel_add_manger",
        json!({ "channel_id": "c1", "users": users.clone() }),
    );
    assert!(url.ends_with("/channel/add/manger"), "url={url}");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["users"], users);

    let (url2, body2) = dispatch(
        "im_channel_remove_manger",
        json!({ "channel_id": "c1", "users": users.clone() }),
    );
    assert!(url2.ends_with("/channel/remove/manger"), "url={url2}");
    assert_eq!(body2["channelId"], "c1");
    assert_eq!(body2["users"], users);
}

#[test]
fn set_manger_routes_endpoint_and_role_by_set_flag() {
    // set=true → ADMIN + channel/add/manger（role 业务赋值 + endpoint 路由下沉 helix）。
    let (url, body) = dispatch(
        "im_channel_set_manger",
        json!({ "channel_id": "c1", "user_id": "u1", "team_id": "t1", "set": true }),
    );
    assert!(url.ends_with("/channel/add/manger"), "url={url}");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["users"][0]["id"], "u1");
    assert_eq!(body["users"][0]["name"], "");
    assert_eq!(body["users"][0]["role"], "ADMIN");
    assert_eq!(body["users"][0]["teamId"], "t1");

    // set=false → MEMBER + channel/remove/manger。
    let (url2, body2) = dispatch(
        "im_channel_set_manger",
        json!({ "channel_id": "c1", "user_id": "u1", "team_id": "t1", "set": false }),
    );
    assert!(url2.ends_with("/channel/remove/manger"), "url={url2}");
    assert_eq!(body2["users"][0]["role"], "MEMBER");

    // 缺 set/user_id → Err（边界零信任，不 panic）。
    let corr = Correlation::from_raw(3);
    let p = serde_json::to_vec(&json!({ "channel_id": "c1", "user_id": "u1" })).unwrap();
    assert!(
        handle_outbound("im_channel_set_manger", &p, "http://h/api", "http://h", None, corr)
            .is_err(),
        "缺 set → Err"
    );
}

#[test]
fn enable_approval_bool_and_channel_id() {
    let (url, body) = dispatch(
        "im_channel_enable_approval",
        json!({ "channel_id": "c1", "approval_status": true }),
    );
    assert!(url.ends_with("/channels/enableApproval"), "url={url}");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["approvalStatus"], true);
}

#[test]
fn view_channels_passthrough_channels_array() {
    let channels = json!([{ "id": "c1", "isRoot": true }, { "id": "c2", "isRoot": false }]);
    let (url, body) = dispatch("im_channels_view", json!({ "channels": channels.clone() }));
    assert!(url.ends_with("/channels/view"), "url={url}");
    assert_eq!(body["channels"], channels);
}

#[test]
fn missing_required_fields_error_not_panic() {
    let corr = Correlation::from_raw(2);
    // displayName 缺 display_name
    let p = serde_json::to_vec(&json!({ "channel_id": "c1" })).unwrap();
    assert!(handle_outbound(
        "im_channel_change_display_name",
        &p,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
    // add/manger 空 users
    let p2 = serde_json::to_vec(&json!({ "channel_id": "c1", "users": [] })).unwrap();
    assert!(handle_outbound(
        "im_channel_add_manger",
        &p2,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
    // enableApproval 缺 bool
    let p3 = serde_json::to_vec(&json!({ "channel_id": "c1" })).unwrap();
    assert!(handle_outbound(
        "im_channel_enable_approval",
        &p3,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
}
