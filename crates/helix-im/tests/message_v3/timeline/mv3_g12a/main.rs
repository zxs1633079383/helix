//! MV3-G12a：回复根分页参数、关联键与单一结构化结果。

use helix_core::effect::Effect;
use helix_im::outbound::handle_outbound;
use helix_im::render_ready_replies::{
    emit_replies_page_result, project_replies_page, ReplyProjectionRequest,
};
use serde_json::{json, Value};

const API_BASE: &str = "http://helix-test/api/cses";
const DEFAULT_BASE: &str = "http://helix-default";
const ROOT: &str = "root-g12a";
const CHANNEL: &str = "channel-g12a";
const VIEWER: &str = "viewer-g12a";
const REQUEST_ID: &str = "req-g12a-1";

/// 从 outbound 命令中取唯一 HTTP effect，作为 I/O 前 wire 合同断言入口。
fn one_http(effects: Vec<Effect>) -> helix_core::effect::HttpRequest {
    let mut http = effects.into_iter().filter_map(|effect| match effect {
        Effect::Http { req, .. } => Some(req),
        _ => None,
    });
    let request = http.next().expect("reply query must issue one HTTP");
    assert!(http.next().is_none(), "reply query must issue one HTTP");
    request
}

/// 构造带请求上下文的 root projection request，避免测试绕过关联字段。
fn request(page_number: u32, page_size: u32) -> ReplyProjectionRequest {
    ReplyProjectionRequest {
        req_id: REQUEST_ID.to_string(),
        channel_id: CHANNEL.to_string(),
        root_hint: ROOT.to_string(),
        mode: helix_im::render_ready_replies::ReplyProjectionMode::Snapshot,
        revision: 1,
        viewer_user_id: VIEWER.to_string(),
        page_number,
        page_size,
    }
}

/// Go getReplies 成功体：包含同 root 的可见/不可见、重复与跨 root 行。
fn replies_body() -> Value {
    json!({
        "status": "SUCCESS",
        "data": {
            "rootPost": {
                "id": ROOT,
                "temporaryId": "tmp-root",
                "channelId": CHANNEL,
                "userId": "author-g12a",
                "type": "TEXT",
                "message": "root",
                "viewers": ["all"],
                "createAt": 1700000000000_i64
            },
            "replies": [
                {
                    "id": "reply-1",
                    "temporaryId": "tmp-reply-1",
                    "rootId": ROOT,
                    "parentId": ROOT,
                    "channelId": CHANNEL,
                    "message": "visible first",
                    "viewers": [VIEWER],
                    "createAt": 1700000001000_i64
                },
                {
                    "id": "reply-1-duplicate",
                    "temporaryId": "tmp-reply-1",
                    "rootId": ROOT,
                    "parentId": ROOT,
                    "channelId": CHANNEL,
                    "message": "duplicate temporary id",
                    "viewers": [VIEWER],
                    "createAt": 1700000002000_i64
                },
                {
                    "id": "reply-hidden",
                    "temporaryId": "tmp-hidden",
                    "rootId": ROOT,
                    "parentId": ROOT,
                    "channelId": CHANNEL,
                    "message": "hidden",
                    "viewers": ["other-viewer"],
                    "createAt": 1700000003000_i64
                },
                {
                    "id": "reply-cross-root",
                    "temporaryId": "tmp-cross-root",
                    "rootId": "other-root",
                    "parentId": "other-root",
                    "channelId": CHANNEL,
                    "message": "must not leak",
                    "viewers": [VIEWER],
                    "createAt": 1700000004000_i64
                }
            ],
            "hasMore": true,
            "page": {"pageNumber": 2, "pageSize": 20}
        }
    })
}

#[test]
fn g12a_outbound_preserves_page_and_req_id_without_extra_root_query() {
    let payload = serde_json::to_vec(&json!({
        "reply_id": ROOT,
        "page_number": 2,
        "page_size": 60,
        "req_id": REQUEST_ID,
        "channel_id": CHANNEL
    }))
    .unwrap();
    let request = one_http(
        handle_outbound(
            "im_get_replies",
            &payload,
            API_BASE,
            DEFAULT_BASE,
            None,
            helix_core::Correlation::from_raw(12),
        )
        .unwrap(),
    );
    assert_eq!(request.url, format!("{API_BASE}/posts/getReplies"));
    let body: Value = serde_json::from_slice(request.body.as_ref().unwrap()).unwrap();
    assert_eq!(body["replyId"], ROOT);
    assert_eq!(body["pageNumber"], 2);
    assert_eq!(body["pageSize"], 60);
    assert_eq!(
        request
            .headers
            .iter()
            .find(|(key, _)| key == "Cses-Track-Id")
            .map(|(_, value)| value.as_str()),
        Some(REQUEST_ID)
    );
    assert!(
        body.get("reqId").is_none(),
        "correlation belongs in the track header"
    );
}

#[test]
fn g12a_rejects_invalid_page_size_before_http() {
    for page_size in [0, 61, -1] {
        let payload = serde_json::to_vec(&json!({
            "reply_id": ROOT,
            "page_number": 1,
            "page_size": page_size,
            "req_id": REQUEST_ID
        }))
        .unwrap();
        let result = handle_outbound(
            "im_get_replies",
            &payload,
            API_BASE,
            DEFAULT_BASE,
            None,
            helix_core::Correlation::from_raw(13),
        );
        assert!(result.is_err(), "pageSize={page_size} must fail before I/O");
    }
}

#[test]
fn g12a_projection_is_root_and_viewer_isolated_and_keeps_authority_page() {
    let projection = project_replies_page(&request(2, 20), &replies_body(), VIEWER).unwrap();
    assert_eq!(projection.req_id, REQUEST_ID);
    assert_eq!(projection.root_post["id"], ROOT);
    assert_eq!(projection.replies.len(), 1);
    assert_eq!(projection.replies[0]["id"], "reply-1");
    assert_eq!(projection.has_more, true);
    assert_eq!(projection.page_number, 2);
    assert_eq!(projection.page_size, 20);
}

#[test]
fn g12a_missing_has_more_is_not_silent_success() {
    let mut body = replies_body();
    body["data"].as_object_mut().unwrap().remove("hasMore");
    let error = project_replies_page(&request(1, 20), &body, VIEWER).unwrap_err();
    assert!(error.to_string().contains("hasMore"));
}

#[test]
fn g12a_success_has_exactly_one_correlated_terminal() {
    let result = project_replies_page(&request(2, 20), &replies_body(), VIEWER).unwrap();
    let Effect::Emit { event } = emit_replies_page_result(&result) else {
        panic!("reply success must emit one terminal result");
    };
    let event: Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(event["event"], "im:read:result");
    assert_eq!(event["data"]["req_id"], REQUEST_ID);
    assert_eq!(event["data"]["body"]["reqId"], REQUEST_ID);
    assert_eq!(event["data"]["body"]["rootPost"]["id"], ROOT);
    assert_eq!(
        event["data"]["body"]["replies"].as_array().unwrap().len(),
        1
    );
    assert_eq!(event["data"]["body"]["hasMore"], true);
}
