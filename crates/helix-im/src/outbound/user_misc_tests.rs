//! `user_misc.rs` 的 12 命令注册 / body / 读写族单测（经 `#[cfg(test)] #[path] mod tests;` 挂回）。
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

/// 12 命令全注册（11 既有 + loadTarget 新增）。
#[test]
fn all_user_misc_commands_registered() {
    for n in [
        "im_list_users",
        "im_get_user_statuses",
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

/// 读写族标注：team 三写 + search_do 为写族；其余为读族。
#[test]
fn read_write_flags_correct() {
    for w in [
        "im_team_upsert",
        "im_team_member_add",
        "im_team_quit",
        "im_search_do",
    ] {
        assert!(!is_read(w), "{w} 应为写族");
    }
    for r in [
        "im_list_users",
        "im_get_user_statuses",
        "im_get_all_modules",
        "im_load_send_notifications",
        "im_load_target_notifications",
        "im_search_post",
    ] {
        assert!(is_read(r), "{r} 应为读族");
    }
}

// ── 既有写族 body 契约回归（camelCase 对齐现网）─────────────────────────────────────

/// teams/upsert：整 team 对象透传（不拆字段）。
#[test]
fn team_upsert_passthrough() {
    let (url, body) = dispatch(
        "im_team_upsert",
        json!({ "team": { "id": "t1", "name": "公司大群", "users": [] } }),
    );
    assert_eq!(url, "http://h/api/cses/teams/upsert");
    assert_eq!(body, json!({ "id": "t1", "name": "公司大群", "users": [] }));
}

/// teams/upsert：缺 team → Err（边界零信任，不 panic）。
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

/// search/post：channelId 必带；user_id 前端下发 → Err（DoSearchRequest.UserId json:"-"）。
#[test]
fn search_post_rejects_client_user_id() {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&json!({ "channel_id": "c1", "user_id": "u1" })).unwrap();
    assert!(handle_outbound(
        "im_search_post",
        &payload,
        "http://h/api/cses",
        "http://h",
        None,
        corr
    )
    .is_err());
}

/// search/post：camelCase body（searchType/teamId/isAll 仅显式给出才下发）。
#[test]
fn search_post_camel_case_body() {
    let (url, body) = dispatch(
        "im_search_post",
        json!({ "channel_id": "c1", "keyword": "hi", "search_type": 1, "is_all": true }),
    );
    assert_eq!(url, "http://h/api/cses/search/post");
    assert_eq!(body["channelId"], json!("c1"));
    assert_eq!(body["keyword"], json!("hi"));
    assert_eq!(body["searchType"], json!(1));
    assert_eq!(body["isAll"], json!(true));
    // 未给的 teamId 不下发（不写 null）。
    assert!(body.get("teamId").is_none());
}
