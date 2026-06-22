//! `commands::handle_outbound` 集成测试 —— 每个 outbound 命令锚现网真源 endpoint url + body。
//!
//! 可证伪（HX-C011）：删/改某命令的 push → 对应断言 FAIL；endpoint url 改错 → FAIL；
//! 缺必填字段 → `Err`（不崩）。从 `src/commands.rs` 抽出（两函数 `pub`，守 src ≤300 行）。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(7)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic。
fn one_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(effects.len(), 1, "outbound 命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        Some("conn-1"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

#[test]
fn read_maps_to_post_read_with_channel_and_endtime() {
    let (url, body) = one_http(run(
        "im_post_read",
        json!({ "channel_id": "ch1", "end_time": 123 }),
    ));
    assert_eq!(url, "http://x/api/cses/post/read");
    assert_eq!(body["channelId"], "ch1");
    assert_eq!(body["endTime"], 123);
}

#[test]
fn revoke_leave_cancel_map_to_real_endpoints() {
    let (url, body) = one_http(run("im_revoke", json!({ "post_id": "p9" })));
    assert_eq!(url, "http://x/api/cses/posts/revoke");
    assert_eq!(body["postId"], "p9");

    let (url, body) = one_http(run("im_channel_leave", json!({ "channel_id": "ch2" })));
    assert_eq!(url, "http://x/api/cses/channel/member/leave");
    assert_eq!(body["channelId"], "ch2");

    let (url, body) = one_http(run("im_cancel_schedule", json!({ "channel_id": "ch3" })));
    assert_eq!(url, "http://x/api/cses/posts/cancelSchedule");
    assert_eq!(body["channelId"], "ch3");
}

#[test]
fn schedule_nests_post_and_schedulepostat() {
    let (url, body) = one_http(run(
        "im_create_schedule",
        json!({ "channel_id": "ch4", "message": "later", "schedule_post_at": 999, "temporary_id": "t1" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/createSchedule");
    assert_eq!(body["schedulePostAt"], 999);
    assert_eq!(body["post"]["channelId"], "ch4");
    assert_eq!(body["post"]["message"], "later");
    assert_eq!(body["post"]["temporaryId"], "t1");
}

#[test]
fn create_and_make_topic_passthrough_body_with_real_endpoint() {
    let (url, body) = one_http(run(
        "im_create_channel",
        json!({ "displayName": "g", "users": [{ "id": "u1" }] }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/create");
    assert_eq!(body["displayName"], "g");

    let (url, body) = one_http(run("im_make_topic", json!({ "postId": "p1" })));
    assert_eq!(url, "http://x/api/cses/posts/makeTopic");
    assert_eq!(body["postId"], "p1");
}

#[test]
fn member_change_passthrough_body_with_real_endpoint() {
    // 真机curl真源 §5：{channelId, joinUsers/leaveUsers} 直传，endpoint channel/member/change。
    let (url, body) = one_http(run(
        "im_channel_member_change",
        json!({
            "channelId": "ch_m",
            "joinUsers": [{ "id": "u9", "teamId": "t1", "role": "MEMBER" }],
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/change");
    assert_eq!(body["channelId"], "ch_m");
    assert_eq!(body["joinUsers"][0]["id"], "u9");

    // 缺 channelId → Err（边界零信任，不崩）。
    assert!(handle_outbound(
        "im_channel_member_change",
        &serde_json::to_vec(&json!({ "joinUsers": [] })).unwrap(),
        BASE,
        None,
        corr(),
    )
    .is_err());
}

#[test]
fn close_and_nickname_map_to_real_endpoints() {
    let (url, body) = one_http(run("im_channel_close", json!({ "channel_id": "ch5" })));
    assert_eq!(url, "http://x/api/cses/channel/close");
    assert_eq!(body["channelId"], "ch5");

    // nickname：含 user_id → 带 userId；nickname 允许任意（含空，Go 侧清空）
    let (url, body) = one_http(run(
        "im_update_member_nickname",
        json!({ "channel_id": "ch6", "nickname": "Bob", "user_id": "u2" }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/change/nickname");
    assert_eq!(body["channelId"], "ch6");
    assert_eq!(body["nickname"], "Bob");
    assert_eq!(body["userId"], "u2");
}

#[test]
fn missing_required_field_is_parse_error_not_panic() {
    // 边界零信任：缺必填 → Err，不崩
    assert!(handle_outbound(
        "im_revoke",
        &serde_json::to_vec(&json!({})).unwrap(),
        BASE,
        None,
        corr(),
    )
    .is_err());
    // 缺 schedule_post_at（int 字段）
    assert!(handle_outbound(
        "im_create_schedule",
        &serde_json::to_vec(&json!({ "channel_id": "c", "message": "m" })).unwrap(),
        BASE,
        None,
        corr(),
    )
    .is_err());
    // channel_close 缺 channel_id
    assert!(handle_outbound(
        "im_channel_close",
        &serde_json::to_vec(&json!({})).unwrap(),
        BASE,
        None,
        corr(),
    )
    .is_err());
}

#[test]
fn is_outbound_covers_exactly_the_ten() {
    for n in [
        "im_post_read",
        "im_revoke",
        "im_channel_leave",
        "im_create_schedule",
        "im_cancel_schedule",
        "im_create_channel",
        "im_make_topic",
        "im_channel_member_change",
        "im_channel_close",
        "im_update_member_nickname",
    ] {
        assert!(is_outbound(n), "{n} 应被认领");
    }
    assert!(!is_outbound("im_send_message"), "send 走 module.rs 不在此");
    assert!(!is_outbound("bogus"));
}
