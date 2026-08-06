//! `user_misc.rs` 的 13 命令注册 / body / 读写族单测（经 `#[cfg(test)] #[path] mod tests;` 挂回）。
//!
//! 从 `user_misc.rs` 外提守 ≤300 硬顶（结构闸 §1，测试外提范本，同 vote_score_tests.rs）。
//! `super::*` 指向 `user_misc` 模块；`handle_outbound`/`is_outbound`/`is_read` 从 registry 取。

use super::*;
use crate::outbound::registry::{handle_outbound, is_outbound, is_read};
use helix_core::{Correlation, Effect};

/// 走 IM 网关（/api/cses）解析首个 Http Effect → (url, body)。
fn dispatch(cmd: &str, body: Value) -> (String, Value) {
    let corr = Correlation::from_raw(9);
    let payload = serde_json::to_vec(&body).unwrap();
    let effects = handle_outbound(cmd, &payload, "http://h/api/cses", "http://h", None, corr)
        .unwrap_or_else(|e| panic!("{cmd} dispatch: {e:?}"));
    match &effects[0] {
        Effect::Http { req, .. } => {
            let parsed: Value =
                serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
            (req.url.clone(), parsed)
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

/// 13 命令全注册（含 loadTarget 与 fresh-account candidate directory）。
#[test]
fn all_user_misc_commands_registered() {
    for n in [
        "im_list_users",
        "im_get_user_statuses",
        "im_user_candidates",
        "im_team_upsert",
        "im_team_member_add",
        "im_team_quit",
        "im_get_all_modules",
        "im_load_send_notifications",
        "im_load_target_notifications",
        "im_search_post",
        "im_search_user",
        "im_search_channel",
        "im_search_do",
    ] {
        assert!(is_outbound(n), "{n} 应注册");
    }
}

/// 读写族标注：team 三写；四个 Search Gate 均通过 req_id 读回。
#[test]
fn read_write_flags_correct() {
    for w in ["im_team_upsert", "im_team_member_add", "im_team_quit"] {
        assert!(!is_read(w), "{w} 应为写族");
    }
    for r in [
        "im_list_users",
        "im_get_user_statuses",
        "im_user_candidates",
        "im_get_all_modules",
        "im_load_send_notifications",
        "im_load_target_notifications",
        "im_search_post",
        "im_search_user",
        "im_search_channel",
        "im_search_do",
    ] {
        assert!(is_read(r), "{r} 应为读族");
    }
}

// ── 既有写族 body 契约回归（camelCase 对齐现网）─────────────────────────────────────

/// teams/upsert：body 成形下沉 helix——壳传结构化 args，helix 拼 team 对象（owner CREATOR）。
#[test]
fn team_upsert_builds_team_object() {
    let (url, body) = dispatch(
        "im_team_upsert",
        json!({ "display_name": "公司大群", "self_id": "u1", "team_id": "t1", "member_ids": [] }),
    );
    assert_eq!(url, "http://h/api/cses/teams/upsert");
    assert_eq!(body["displayName"], "公司大群");
    assert_eq!(body["teamId"], "t1");
    assert_eq!(body["type"], "P");
    assert_eq!(body["owner"]["role"], "CREATOR");
    assert_eq!(body["users"][0]["role"], "CREATOR");
    assert_eq!(body["forceCreate"], true);
}

/// teams/upsert：缺 display_name/self_id → Err（边界零信任，不 panic）。
#[test]
fn team_upsert_rejects_missing() {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&json!({})).unwrap();
    assert!(handle_outbound(
        "im_team_upsert",
        &payload,
        "http://h/api/cses",
        "http://h",
        None,
        corr
    )
    .is_err());
}

/// teams/member/add：整 ChannelMemberEvent 透传。
#[test]
fn team_member_add_passthrough() {
    let (url, body) = dispatch(
        "im_team_member_add",
        json!({ "event": { "channelId": "c1", "joinUsers": [], "leaveUsers": [] } }),
    );
    assert_eq!(url, "http://h/api/cses/teams/member/add");
    assert_eq!(
        body,
        json!({ "channelId": "c1", "joinUsers": [], "leaveUsers": [] })
    );
}

/// teams/member/quit：{userId, teamId}（camelCase）。
#[test]
fn team_quit_body() {
    let (url, body) = dispatch("im_team_quit", json!({ "user_id": "u1", "team_id": "t1" }));
    assert_eq!(url, "http://h/api/cses/teams/member/quit");
    assert_eq!(body, json!({ "userId": "u1", "teamId": "t1" }));
}

/// teams/member/quit：缺 team_id → Err。
#[test]
fn team_quit_rejects_missing() {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&json!({ "user_id": "u1" })).unwrap();
    assert!(handle_outbound(
        "im_team_quit",
        &payload,
        "http://h/api/cses",
        "http://h",
        None,
        corr
    )
    .is_err());
}

// ── notification 姊妹族 ────────────────────────────────────────────────────────────

/// loadSend：{page,size,state?}（state 可选，给则透传）。
#[test]
fn load_send_body_with_state() {
    let (url, body) = dispatch(
        "im_load_send_notifications",
        json!({ "page": 1, "size": 20, "state": "unread" }),
    );
    assert_eq!(url, "http://h/api/cses/notification/loadSend");
    assert_eq!(body, json!({ "page": 1, "size": 20, "state": "unread" }));
}

/// loadTarget：{page,size}（无 state 字段，给了也不带）；显式值透传。
#[test]
fn load_target_explicit_page_size() {
    let (url, body) = dispatch(
        "im_load_target_notifications",
        json!({ "page": 3, "size": 50, "state": "ignored" }),
    );
    assert_eq!(url, "http://h/api/cses/notification/loadTarget");
    // 无 state 字段（loadTarget req=query.Page，对齐源 notification.go:40）。
    assert_eq!(body, json!({ "page": 3, "size": 50 }));
}

/// loadTarget：缺省取 Go handler 同款默认 {page:0,size:10}（notification.go:40-41 pre-init）。
#[test]
fn load_target_defaults_match_go_handler() {
    let (url, body) = dispatch("im_load_target_notifications", json!({}));
    assert_eq!(url, "http://h/api/cses/notification/loadTarget");
    assert_eq!(body, json!({ "page": 0, "size": 10 }));
}

// ── search 契约 ────────────────────────────────────────────────────────────────────

/// search/post：服务端权威字段即使其余入参有效也必须 fail-closed。
#[test]
fn search_rejects_client_authority_and_sort_fields() {
    for field in ["user_id", "team_id", "company_id", "sort"] {
        let mut body = json!({
            "req_id": "rq-1",
            "scope": "channel",
            "channel_id": "c1",
            "keyword": "hi"
        });
        body[field] = json!("forbidden");
        let payload = serde_json::to_vec(&body).unwrap();
        assert!(
            handle_outbound(
                "im_search_post",
                &payload,
                "http://h/api/cses",
                "http://h",
                None,
                Correlation::from_raw(1)
            )
            .is_err(),
            "{field} 必须拒绝"
        );
    }
}

/// search/post：全部筛选精确转为 camelCase，correlation 字段不进入 wire。
#[test]
fn search_post_camel_case_body() {
    let (url, body) = dispatch(
        "im_search_post",
        json!({
            "req_id": "rq-1",
            "scope": "channel",
            "channel_id": "c1",
            "keyword": " hi ",
            "sender_ids": ["u1", "u2"],
            "content_types": ["FILE", "IMAGE"],
            "start_at": 10,
            "end_at": 20,
            "cursor": "next-1",
            "page_size": 25
        }),
    );
    assert_eq!(url, "http://h/api/cses/search/post");
    assert_eq!(body["scope"], json!("channel"));
    assert_eq!(body["channelId"], json!("c1"));
    assert_eq!(body["keyword"], json!("hi"));
    assert_eq!(body["senderIds"], json!(["u1", "u2"]));
    assert_eq!(body["contentTypes"], json!(["FILE", "IMAGE"]));
    assert_eq!(body["startAt"], json!(10));
    assert_eq!(body["endAt"], json!(20));
    assert_eq!(body["cursor"], json!("next-1"));
    assert_eq!(body["pageSize"], json!(25));
    assert!(body.get("reqId").is_none());
}

/// search/post：允许纯筛选查询，并拒绝缺 scope、空查询、非法时间与越界分页。
#[test]
fn search_post_filter_and_boundaries_fail_closed() {
    let (_, body) = dispatch(
        "im_search_post",
        json!({
            "req_id": "rq-filter",
            "scope": "channel",
            "channel_id": "c1",
            "content_types": ["FILE"]
        }),
    );
    assert_eq!(body["contentTypes"], json!(["FILE"]));
    for invalid in [
        json!({ "req_id": "r", "channel_id": "c1", "keyword": "x" }),
        json!({ "req_id": "r", "scope": "channel", "keyword": "x" }),
        json!({ "req_id": "r", "scope": "global" }),
        json!({ "req_id": "r", "scope": "global", "keyword": "x", "start_at": 20, "end_at": 10 }),
        json!({ "req_id": "r", "scope": "global", "keyword": "x", "page_size": 51 }),
        json!({ "req_id": "r", "scope": "global", "keyword": "x", "cursor": "" }),
    ] {
        let payload = serde_json::to_vec(&invalid).unwrap();
        assert!(handle_outbound(
            "im_search_post",
            &payload,
            "http://h/api/cses",
            "http://h",
            None,
            Correlation::from_raw(2)
        )
        .is_err());
    }
}

/// user/channel/do：共享必填 keyword、有界 cursor/page_size 与本地 req_id 合同。
#[test]
fn keyword_search_commands_share_bounded_wire_contract() {
    for (name, endpoint) in [
        ("im_search_user", "search/user"),
        ("im_search_channel", "search/channel"),
        ("im_search_do", "search/do"),
    ] {
        let (url, body) = dispatch(
            name,
            json!({
                "req_id": "rq-keyword",
                "keyword": " 张三 ",
                "cursor": "cursor-1",
                "page_size": 50
            }),
        );
        assert_eq!(url, format!("http://h/api/cses/{endpoint}"));
        assert_eq!(
            body,
            json!({ "keyword": "张三", "cursor": "cursor-1", "pageSize": 50 })
        );
    }
}

/// 四个搜索命令缺失 req_id 时必须在发 HTTP 前拒绝，避免 UI promise 永久挂起。
#[test]
fn search_commands_require_req_id() {
    for name in [
        "im_search_post",
        "im_search_user",
        "im_search_channel",
        "im_search_do",
    ] {
        let body = if name == "im_search_post" {
            json!({ "scope": "global", "keyword": "x" })
        } else {
            json!({ "keyword": "x" })
        };
        let payload = serde_json::to_vec(&body).unwrap();
        assert!(handle_outbound(
            name,
            &payload,
            "http://h/api/cses",
            "http://h",
            None,
            Correlation::from_raw(3)
        )
        .is_err());
    }
}
