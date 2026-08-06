//! `commands::handle_outbound` 集成测试 —— 每个 outbound 命令锚现网真源 endpoint url + body。
//!
//! 可证伪（HX-C011）：删/改某命令的 push → 对应断言 FAIL；endpoint url 改错 → FAIL；
//! 缺必填字段 → `Err`（不崩）。从 `src/commands.rs` 抽出（两函数 `pub`，守 src ≤300 行）。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";
const CANONICAL_CHANNEL: &str = "chfixx00000000000000000001";
const CANONICAL_POST_1: &str = "srvfix00000000000000000001";
const CANONICAL_POST_2: &str = "srvfix00000000000000000002";

fn corr() -> Correlation {
    Correlation::from_raw(7)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic。
fn one_http_with_headers(effects: Vec<Effect>) -> (String, Value, Vec<(String, String)>) {
    assert_eq!(effects.len(), 1, "outbound 命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body, req.headers)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn one_http(effects: Vec<Effect>) -> (String, Value) {
    let (url, body, _) = one_http_with_headers(effects);
    (url, body)
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        Some("conn-1"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

fn fails(name: &str, payload: Value) -> bool {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err()
}

/// interval 字段不属于严格 `{channel_id,posts}` 合同，必须在 HTTP 前拒绝。
#[test]
fn read_rejects_interval_only_payload_before_http() {
    assert!(fails(
        "im_post_read",
        json!({
            "channel_id": CANONICAL_CHANNEL,
            "start_time": 100,
            "end_time": 123
        }),
    ));
}

/// UC-3.2 单条/列表已读：前端 onPostRead 发 `{channel_id, posts:[postId]}`（按 posts 列表标记单条已读）。
/// 真源 entity.PostRead.Posts []string `json:"posts,omitempty"`（post.go:381 / full-map #23）。
/// 偏差修复前 builder 从不构建 posts → Go ReadPost 收到无 posts 无区间 → 标记错/不标记。
#[test]
fn read_carries_posts_list_for_single_post_read() {
    let (url, body) = one_http(run(
        "im_post_read",
        json!({
            "channel_id": CANONICAL_CHANNEL,
            "posts": [CANONICAL_POST_1, CANONICAL_POST_2]
        }),
    ));
    assert_eq!(url, "http://x/api/cses/post/read");
    assert_eq!(
        body,
        json!({
            "channelId": CANONICAL_CHANNEL,
            "posts": [CANONICAL_POST_1, CANONICAL_POST_2]
        })
    );
}

/// 空、非字符串及非 canonical ID 均须在 HTTP 前拒绝，禁止静默降级为区间读取。
#[test]
fn read_rejects_empty_invalid_or_noncanonical_payload_before_http() {
    assert!(fails(
        "im_post_read",
        json!({ "channel_id": CANONICAL_CHANNEL, "posts": [] }),
    ));

    assert!(fails(
        "im_post_read",
        json!({ "channel_id": CANONICAL_CHANNEL, "posts": "x" }),
    ));

    assert!(fails(
        "im_post_read",
        json!({ "channel_id": "ch1", "posts": [CANONICAL_POST_1] }),
    ));
    assert!(fails(
        "im_post_read",
        json!({ "channel_id": CANONICAL_CHANNEL, "posts": ["p-1"] }),
    ));
}

#[test]
fn revoke_leave_cancel_map_to_real_endpoints() {
    let (url, body) = one_http(run("im_revoke", json!({ "post_id": "p9" })));
    assert_eq!(url, "http://x/api/cses/posts/revoke");
    assert_eq!(body["postId"], "p9");

    let (url, body) = one_http(run("im_channel_leave", json!({ "channel_id": "ch2" })));
    assert_eq!(url, "http://x/api/cses/channel/member/leave");
    assert_eq!(body["channelId"], "ch2");
    // 仅自己退、无转让：不发空 userId/newCreator（Go *string / *struct omitempty）
    assert!(body.get("userId").is_none(), "无 user_id 不进 body");
    assert!(body.get("newCreator").is_none(), "无 new_creator 不进 body");

    let (url, body) = one_http(run("im_cancel_schedule", json!({ "channel_id": "ch3" })));
    assert_eq!(url, "http://x/api/cses/posts/cancelSchedule");
    assert_eq!(body["channelId"], "ch3");
}

/// UC-5.3 群主退群转让：full-map #33 entity.ChannelIdStruct{channelId, userId *string,
/// newCreator *ChannelMemberMinimal}；L596「NewCreator 用于群主退群时转让群主」。
/// 偏差修复前 build 只发 channelId，静默丢 newCreator/userId → 群主转让失效。
/// 透传后 wire key 必须是 camelCase `newCreator` / `userId`（对齐 Go json tag）。
#[test]
fn leave_carries_new_creator_and_user_id_for_owner_transfer() {
    let (url, body) = one_http(run(
        "im_channel_leave",
        json!({
            "channel_id": "ch9",
            "user_id": "u-self",
            "new_creator": { "id": "u-heir", "teamId": "t1", "role": 0 }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/leave");
    assert_eq!(body["channelId"], "ch9");
    assert_eq!(body["userId"], "u-self");
    // newCreator 整对象透传（ChannelMemberMinimal 结构由前端直供，build 不解构）
    assert_eq!(body["newCreator"]["id"], "u-heir");
    assert_eq!(body["newCreator"]["teamId"], "t1");
    // 命名陷阱：是 newCreator（camelCase），不是 snake new_creator
    assert!(
        body.get("new_creator").is_none(),
        "wire key 为 camelCase newCreator"
    );
}

#[test]
/// 固定 G08 wire nesting，并拒绝调用方尝试注入 Helix-owned temporaryId。
fn schedule_nests_post_and_schedulepostat() {
    let (url, body) = one_http(run(
        "im_create_schedule",
        json!({
            "channel_id": "ch4",
            "message": "later",
            "type": "CUSTOM",
            "props": { "source": "mobile", "priority": 2 },
            "schedule_post_at": 999
        }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/createSchedule");
    assert_eq!(body["schedulePostAt"], 999);
    assert_eq!(body["post"]["channelId"], "ch4");
    assert_eq!(body["post"]["message"], "later");
    assert_eq!(body["post"]["type"], "CUSTOM");
    assert_eq!(body["post"]["props"]["source"], "mobile");
    assert_eq!(body["post"]["props"]["priority"], 2);
    assert!(body["post"].get("temporaryId").is_none());
    assert!(fails(
        "im_create_schedule",
        json!({
            "channel_id": "ch4",
            "message": "later",
            "schedule_post_at": 999,
            "temporary_id": "legacy-must-not-enter"
        })
    ));
}

#[test]
fn schedule_requires_positive_time_and_object_props() {
    assert!(fails(
        "im_create_schedule",
        json!({ "channel_id": "ch4", "message": "later", "schedule_post_at": 0 })
    ));
    assert!(fails(
        "im_create_schedule",
        json!({ "channel_id": "ch4", "message": "later", "schedule_post_at": -1 })
    ));
    assert!(fails(
        "im_create_schedule",
        json!({ "channel_id": "ch4", "message": "later", "schedule_post_at": 1, "props": [] })
    ));
}

#[test]
fn create_and_make_topic_build_body_with_real_endpoint() {
    let (url, body, headers) = one_http_with_headers(run(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": ["u1"], "display_name": "g",
            "force_create": true, "req_id": "create-request-1"
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/create");
    assert_eq!(
        body,
        json!({
            "type": "O", "userIds": ["u1"], "displayName": "g",
            "forceCreate": true
        })
    );
    assert!(headers
        .iter()
        .any(|(key, value)| key == "Cses-Track-Id" && value == "create-request-1"));
    assert!(
        body.get("req_id").is_none(),
        "req_id 只能进入 transport header"
    );

    assert!(fails(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": ["u1"], "display_name": "g",
            "force_create": true, "req_id": ""
        })
    ));

    let (url, body) = one_http(run(
        "im_make_topic",
        json!({ "root_id": "r1", "user_ids": ["u1"], "req_id": "topic-request-1" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/makeTopic");
    assert_eq!(
        body,
        json!({
            "rootId": "r1", "userIds": ["u1"]
        })
    );

    // 缺 req_id → Err（边界零信任，不崩）。
    assert!(handle_outbound(
        "im_make_topic",
        &serde_json::to_vec(&json!({ "root_id": "r1", "user_ids": ["u1"] })).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err());
}

#[test]
fn leave_keeps_request_id_in_transport_header_only() {
    let (url, body, headers) = one_http_with_headers(run(
        "im_channel_leave",
        json!({ "channel_id": "ch2", "req_id": "leave-request-1" }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/leave");
    assert_eq!(body, json!({ "channelId": "ch2" }));
    assert!(headers
        .iter()
        .any(|(key, value)| key == "Cses-Track-Id" && value == "leave-request-1"));
}

#[test]
fn member_change_build_body_with_real_endpoint() {
    // body 成形下沉 helix：壳传 {channel_id, team_id, self_id, join_user_ids}，helix 拼 joinUsers[]。
    let (url, body) = one_http(run(
        "im_channel_member_change",
        json!({
            "channel_id": "ch_m",
            "team_id": "t1",
            "self_id": "me",
            "join_user_ids": ["u9", "me"],
            "leave_user_ids": [],
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/change");
    assert_eq!(body["channelId"], "ch_m");
    // join 排除自身（me 被过滤），仅 u9 入 joinUsers，role=MEMBER。
    assert_eq!(body["joinUsers"].as_array().unwrap().len(), 1);
    assert_eq!(body["joinUsers"][0]["id"], "u9");
    assert_eq!(body["joinUsers"][0]["role"], "MEMBER");
    assert_eq!(body["joinUsers"][0]["teamId"], "t1");
    // 空 leave 集 → 字段省略（不发空数组）。
    assert!(body.get("leaveUsers").is_none());

    // 缺 channel_id → Err（边界零信任，不崩）。
    assert!(handle_outbound(
        "im_channel_member_change",
        &serde_json::to_vec(&json!({ "join_user_ids": [] })).unwrap(),
        BASE,
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
