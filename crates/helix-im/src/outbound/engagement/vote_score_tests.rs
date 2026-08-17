//! `vote_score.rs` 的 14 命令注册 / body / 读写族单测（经 `#[cfg(test)] #[path] mod tests;` 挂回）。
//!
//! 从 `vote_score.rs` 外提守 ≤300 硬顶（结构闸 §1）——测试外提范本（同 state_tests.rs）。
//! `super::*` 指向 `vote_score` 模块，故 `is_outbound`/`handle_outbound` 等路径与内联时一致。

use super::*;
use crate::outbound::registry::{handle_outbound, is_outbound, is_read};
use helix_core::{Correlation, Effect};

/// 14 命令全注册 + 读写族正确（read=readVote/average-read；其余 12 写族）。
#[test]
fn fourteen_commands_registered_with_read_flags() {
    for n in [
        "im_vote_create",
        "im_vote_read",
        "im_vote_delete",
        "im_average_read",
        "im_average_delete",
        "im_vote_do",
        "im_vote_close",
        "im_average_attend",
        "im_average_close",
        "im_average_publish",
        "im_vote_update_finish_time",
        "im_vote_add_members",
        "im_average_modify_cutoff",
        "im_average_add_member",
    ] {
        assert!(is_outbound(n), "{n} 应注册");
    }
    // 仅两个读族（HTTP 响应体即数据 → 回灌前端）。
    assert!(is_read("im_vote_read"));
    assert!(is_read("im_average_read"));
    // 其余 8 全写族（写后无数据体回灌，靠 WS 回声 / 前端自刷）。
    for n in [
        "im_vote_create",
        "im_vote_delete",
        "im_average_delete",
        "im_vote_do",
        "im_vote_close",
        "im_average_attend",
        "im_average_close",
        "im_average_publish",
        "im_vote_update_finish_time",
        "im_vote_add_members",
        "im_average_modify_cutoff",
        "im_average_add_member",
    ] {
        assert!(!is_read(n), "{n} 应为写族");
    }
}

/// 各写命令走第二网关（base = default_api_base_url，非 /api/cses IM 网关）。
fn build_http(cmd: &str, body: Value) -> (String, Value) {
    let corr = Correlation::from_raw(9);
    let payload = serde_json::to_vec(&body).unwrap();
    let effects = handle_outbound(
        cmd,
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
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

/// vote/vote：id+indexes 进 body，postId 存在则透传。
#[test]
fn vote_do_body_with_optional_post_id() {
    let (url, body) = build_http(
        "im_vote_do",
        json!({ "id": "v1", "indexes": ["0", "2"], "postId": "p9" }),
    );
    assert_eq!(url, "http://localhost:3399/vote/vote");
    assert_eq!(body["id"], json!("v1"));
    assert_eq!(body["indexes"], json!(["0", "2"]));
    assert_eq!(body["postId"], json!("p9"));
}

/// 当前 Angular→Host 载荷使用 snake_case；Helix 必须还原 Java 的 postId wire 键。
#[test]
fn vote_do_accepts_snake_case_post_id() {
    let (_, body) = build_http(
        "im_vote_do",
        json!({ "id": "v1", "indexes": ["0"], "post_id": "p9", "req_id": "r1" }),
    );
    assert_eq!(body["postId"], json!("p9"));
    assert!(body.get("post_id").is_none());
    assert!(body.get("req_id").is_none());
}

/// 稳定 mutation req_id 只进入 Cses-Track-Id 关联头，不进入 Java 业务 body。
#[test]
fn engagement_req_id_is_transport_only_and_reaches_track_header() {
    let corr = Correlation::from_raw(11);
    let payload = serde_json::to_vec(&json!({
        "id": "v1",
        "indexes": ["0"],
        "post_id": "p9",
        "req_id": "mv3-vote-do-stable-1",
        "client_mutation_id": "mv3-vote-do-stable-1",
    }))
    .unwrap();
    let effects = handle_outbound(
        "im_vote_do",
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
    .expect("engagement dispatch");
    match &effects[0] {
        Effect::Http { req, .. } => {
            let track = req
                .headers
                .iter()
                .find(|(name, _)| name == "Cses-Track-Id")
                .map(|(_, value)| value.as_str());
            assert_eq!(track, Some("mv3-vote-do-stable-1"));
            let body: Value = serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
            assert!(body.get("req_id").is_none());
            assert!(body.get("clientMutationId").is_none());
            assert_eq!(body["postId"], json!("p9"));
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

/// vote/vote：postId 缺省时 body 不含该键（不写 null/空）。
#[test]
fn vote_do_omits_absent_post_id() {
    let (_, body) = build_http("im_vote_do", json!({ "id": "v1", "indexes": ["0"] }));
    assert!(body.get("postId").is_none(), "缺省 postId 不应入 body");
}

/// vote/vote：indexes 缺失/非 array → Err（边界零信任）。
#[test]
fn vote_do_missing_indexes_errs() {
    let corr = Correlation::from_raw(3);
    let payload = serde_json::to_vec(&json!({ "id": "v1" })).unwrap();
    assert!(handle_outbound(
        "im_vote_do",
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
    .is_err());
}

/// vote/closeVote：{ id } body。
#[test]
fn vote_close_id_body() {
    let (url, body) = build_http("im_vote_close", json!({ "id": "v7" }));
    assert_eq!(url, "http://localhost:3399/vote/closeVote");
    assert_eq!(body, json!({ "id": "v7" }));
}

/// average/attend：id+score(number) 进 body，postId 可选透传。
#[test]
fn average_attend_body_with_score() {
    let (url, body) = build_http(
        "im_average_attend",
        json!({ "id": "a1", "score": 4.5, "postId": "p3" }),
    );
    assert_eq!(url, "http://localhost:3399/average/attend");
    assert_eq!(body["id"], json!("a1"));
    assert_eq!(body["score"], json!(4.5));
    assert_eq!(body["postId"], json!("p3"));
}

/// average/attend：score 非 number（字符串）→ Err。
#[test]
fn average_attend_non_number_score_errs() {
    let corr = Correlation::from_raw(4);
    let payload = serde_json::to_vec(&json!({ "id": "a1", "score": "5" })).unwrap();
    assert!(handle_outbound(
        "im_average_attend",
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
    .is_err());
}

/// average/close：id 必填，postId 缺省不入 body。
#[test]
fn average_close_omits_absent_post_id() {
    let (url, body) = build_http("im_average_close", json!({ "id": "a9" }));
    assert_eq!(url, "http://localhost:3399/average/close");
    assert_eq!(body, json!({ "id": "a9" }));
}

/// average/publish：字段多 → 整 args 透传（不丢字段）。
#[test]
fn average_publish_passthrough_body() {
    let (url, body) = build_http(
        "im_average_publish",
        json!({
            "title": "Q3 评分",
            "content": "请评分",
            "maxScore": "100",
            "minScore": "0",
            "isDelMaxMin": true,
            "isAnonymous": false,
            "cutoff": 1700000000,
            "members": [{ "userId": "u1", "userName": "Bob" }],
        }),
    );
    assert_eq!(url, "http://localhost:3399/average/publish");
    assert_eq!(body["title"], json!("Q3 评分"));
    assert_eq!(
        body["members"],
        json!([{ "userId": "u1", "userName": "Bob" }])
    );
    assert_eq!(body["isDelMaxMin"], json!(true));
}

/// 发布评分不应把 Host 关联键送到 Java，成员 snake_case 仍需还原为 Java wire。
#[test]
fn average_publish_strips_transport_and_restores_nested_keys() {
    let (url, body) = build_http(
        "im_average_publish",
        json!({
            "title": "Q3 评分",
            "max_score": "100",
            "min_score": "0",
            "members": [{ "user_id": "u1", "user_name": "Bob" }],
            "req_id": "r1",
        }),
    );
    assert_eq!(url, "http://localhost:3399/average/publish");
    assert_eq!(body["maxScore"], json!("100"));
    assert_eq!(
        body["members"],
        json!([{ "userId": "u1", "userName": "Bob" }])
    );
    assert!(body.get("req_id").is_none());
}

/// vote/updateFinishTime：snake_case 入参转换为 Java 的 finishTime。
#[test]
fn vote_update_finish_time_body() {
    let (url, body) = build_http(
        "im_vote_update_finish_time",
        json!({ "id": "v1", "finish_time": 1700000000000_i64 }),
    );
    assert_eq!(url, "http://localhost:3399/vote/updateFinishTime");
    assert_eq!(body, json!({ "id": "v1", "finishTime": 1700000000000_i64 }));
}

/// vote/addMembersToVote：只提交 id 与 userIds，不把客户端展示字段送给 Java。
#[test]
fn vote_add_members_body() {
    let (url, body) = build_http(
        "im_vote_add_members",
        json!({ "id": "v1", "user_ids": ["u1", "u2"] }),
    );
    assert_eq!(url, "http://localhost:3399/vote/addMembersToVote");
    assert_eq!(body, json!({ "id": "v1", "userIds": ["u1", "u2"] }));
}

/// average/modifyCutoff：截止时间必须是 JSON number。
#[test]
fn average_modify_cutoff_body() {
    let (url, body) = build_http(
        "im_average_modify_cutoff",
        json!({ "id": "a1", "cutoff": 1700000000000_i64 }),
    );
    assert_eq!(url, "http://localhost:3399/average/modifyCutoff");
    assert_eq!(body, json!({ "id": "a1", "cutoff": 1700000000000_i64 }));
}

/// average/addMember：成员嵌套键递归转换为 Java AddMemberCommand 形状。
#[test]
fn average_add_member_body() {
    let (url, body) = build_http(
        "im_average_add_member",
        json!({
            "id": "a1",
            "members": [{ "user_id": "u1", "user_name": "Alice" }]
        }),
    );
    assert_eq!(url, "http://localhost:3399/average/addMember");
    assert_eq!(
        body,
        json!({ "id": "a1", "members": [{ "userId": "u1", "userName": "Alice" }] })
    );
}

/// 截止时间与成员变更的类型错误必须 fail-closed，避免把半成品请求发给 Java。
#[test]
fn lifecycle_update_commands_reject_invalid_shapes() {
    let corr = Correlation::from_raw(12);
    for (command, body) in [
        (
            "im_vote_update_finish_time",
            json!({ "id": "v1", "finish_time": "later" }),
        ),
        (
            "im_vote_add_members",
            json!({ "id": "v1", "user_ids": "u1" }),
        ),
        (
            "im_average_modify_cutoff",
            json!({ "id": "a1", "cutoff": "later" }),
        ),
        (
            "im_average_add_member",
            json!({ "id": "a1", "members": {} }),
        ),
    ] {
        let payload = serde_json::to_vec(&body).expect("test payload");
        assert!(
            handle_outbound(
                command,
                &payload,
                "http://h/api/cses",
                "http://localhost:3399",
                None,
                corr,
            )
            .is_err(),
            "{command} 应拒绝错误字段形状"
        );
    }
}

/// createVote 整 args 透传：body 逐字段等于入参（不丢字段）。
#[test]
fn create_vote_passthrough_body() {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&json!({
        "fromUserId": "u1",
        "fromUserName": "Alice",
        "title": "lunch?",
        "options": ["A", "B"],
        "isReal": false,
        "req_id": "r1",
    }))
    .unwrap();
    let effects = handle_outbound(
        "im_vote_create",
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
    .expect("create dispatch");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert_eq!(req.url, "http://localhost:3399/vote/createVote");
            let body: Value = serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
            assert_eq!(body["fromUserName"], json!("Alice"));
            assert_eq!(body["options"], json!(["A", "B"]));
            assert!(body.get("req_id").is_none());
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

/// id 缺失 → Err（不 panic，边界零信任）。
#[test]
fn read_vote_missing_id_errs() {
    let corr = Correlation::from_raw(2);
    let payload = serde_json::to_vec(&json!({})).unwrap();
    assert!(handle_outbound(
        "im_vote_read",
        &payload,
        "http://h/api/cses",
        "http://localhost:3399",
        None,
        corr,
    )
    .is_err());
}
