//! `render_ready_replies` 单测（TDD·issue #57 S8·终局切）。sibling 守 ≤300 行硬顶。

use super::*;

fn emit_json(e: Effect) -> serde_json::Value {
    match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
        _ => panic!("expected Emit"),
    }
}

#[test]
fn getreplies_body_extracts_rootpost_then_replies_in_order() {
    // getReplies 形态：{rootPost, replies:[Post]}（partial 1 §15）。
    let body = serde_json::json!({
        "rootPost": {"id": "root1", "message": "根"},
        "replies": [
            {"id": "r1"},
            {"id": "r2"},
            {"id": "r1"},          // 重复 → 去重保首
            {"message": "缺 id"},  // 缺 id → 跳过
        ]
    });
    let tree = extract_flat_replies(&body);
    assert_eq!(tree.root_message_id, "root1");
    assert_eq!(tree.reply_ids, vec!["r1", "r2"]);
    assert_eq!(tree.reply_count, 2);
    assert_eq!(tree.nodes[0]["msgId"], "root1");
}

#[test]
fn common_response_envelope_is_unwrapped_for_replies_and_branch() {
    let replies = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "rootPost": {"id": "root"},
            "replies": [{"id": "first"}]
        }
    });
    let tree = extract_flat_replies(&replies);
    assert_eq!(tree.root_message_id, "root");
    assert_eq!(tree.reply_ids, vec!["first"]);

    let branch = serde_json::json!({
        "status": "SUCCESS",
        "data": {"data": [{"id": "nested"}], "hasMore": false}
    });
    assert_eq!(extract_flat_replies(&branch).reply_ids, vec!["nested"]);
}

#[test]
fn getreplybranch_body_data_and_list_and_toplevel_array() {
    // getReplyBranch 形态：HasMorePage[Post]（{data:[]} / {list:[]} / 顶层数组）。
    let data_form = serde_json::json!({"data": [{"id": "a"}, {"id": "b"}]});
    assert_eq!(extract_flat_replies(&data_form).reply_ids, vec!["a", "b"]);

    let list_form = serde_json::json!({"list": [{"id": "c"}]});
    assert_eq!(extract_flat_replies(&list_form).reply_ids, vec!["c"]);

    let top_array = serde_json::json!([{"id": "x"}, {"id": "y"}, {"id": "x"}]);
    assert_eq!(extract_flat_replies(&top_array).reply_ids, vec!["x", "y"]);
}

#[test]
fn malformed_or_error_body_yields_empty() {
    // 失败回灌 {req_id, error} 无 reply 形态 / 非对象非数组 → 空（前端 reject·清抽屉）。
    assert!(extract_flat_replies(&serde_json::json!({"error": "boom"}))
        .reply_ids
        .is_empty());
    assert!(extract_flat_replies(&serde_json::Value::Null)
        .reply_ids
        .is_empty());
    assert!(extract_flat_replies(&serde_json::json!("plain string"))
        .reply_ids
        .is_empty());
    assert!(extract_flat_replies(&serde_json::json!(42))
        .reply_ids
        .is_empty());
}

#[test]
fn health_body_status_not_treated_as_reply() {
    // UC-12.1 健康探针 body {status:"OK"} 共用 im:read:result·无 reply 形态 → replyIds 空（不误塞）。
    assert!(extract_flat_replies(&serde_json::json!({"status": "OK"}))
        .reply_ids
        .is_empty());
}

#[test]
fn emit_channel_replies_shape_camelcase() {
    let e = emit_channel_replies(
        "rq-1",
        FlatReplyProjection {
            channel_id: "channel-1".into(),
            root_message_id: "root".into(),
            projection_mode: ReplyProjectionMode::Snapshot,
            revision: 7,
            reply_ids: vec!["p1".into(), "p2".into()],
            nodes: vec![serde_json::json!({"msgId": "root"})],
            reply_count: 2,
            has_more: true,
        },
    );
    let v = emit_json(e);
    assert_eq!(v["event"], "im:channel:replies");
    assert_eq!(v["data"]["reqId"], "rq-1");
    assert_eq!(v["data"]["replyIds"][0], "p1");
    assert_eq!(v["data"]["replyIds"][1], "p2");
    assert_eq!(v["data"]["rootMessageId"], "root");
    assert_eq!(v["data"]["channelId"], "channel-1");
    assert_eq!(v["data"]["projectionMode"], "snapshot");
    assert_eq!(v["data"]["revision"], 7);
    assert_eq!(v["data"]["replyCount"], 2);
    assert!(v["data"]["cursor"].is_null());
    assert_eq!(v["data"]["hasMore"], true);
    assert_eq!(v["data"]["nodes"][0]["msgId"], "root");
}

#[test]
fn emit_channel_replies_empty_clears_drawer() {
    // 失败/空 → replyIds:[]（壳 applyChannelReplies set([]) 清抽屉·reject 语义）。
    let v = emit_json(emit_channel_replies("rq-2", FlatReplyProjection::empty()));
    assert_eq!(v["data"]["reqId"], "rq-2");
    assert!(v["data"]["replyIds"].as_array().unwrap().is_empty());
}

#[test]
fn reply_tree_is_single_pass_flat_projection() {
    let body = serde_json::json!({
        "rootPost": {
            "id":"root",
            "message":"原消息",
            "replyCount": 4,
            "repliedMessage": {"id":"quoted", "message":"被引用"},
            "replyMessages": [{"id":"r1", "message":"一级"}]
        },
        "replies": [
            {"id":"r1", "channelId":"c1", "userId":"u1", "message":"一级", "replyId":"root", "replyRootId":"root"},
            {"id":"r2", "channelId":"c1", "userId":"u2", "message":"二级", "replyId":"r1", "replyRootId":"root", "replyFirstLevelId":"r1"}
        ]
    });
    let tree = extract_flat_replies(&body);
    assert_eq!(tree.reply_count, 4, "服务端总数大于本页长度时保留权威总数");
    assert_eq!(tree.reply_ids, vec!["r1", "r2"]);
    assert_eq!(tree.nodes[1]["replyRootId"], "root");
    assert_eq!(tree.nodes[2]["replyFirstLevelId"], "r1");
    assert_eq!(tree.nodes[2]["text"], "二级");
    assert_eq!(tree.nodes[0]["replyCount"], 4);
    assert_eq!(tree.nodes[0]["repliedMessage"]["id"], "quoted");
    assert_eq!(tree.nodes[0]["replyMessages"][0]["id"], "r1");
}

#[test]
fn command_semantics_are_explicit_and_not_inferred_from_req_id() {
    let snapshot = ReplyProjectionRequest::from_command(
        "im_get_replies",
        br#"{"req_id":"same","channel_id":"c1","reply_id":"root"}"#,
        8,
    )
    .unwrap();
    let append = ReplyProjectionRequest::from_command(
        "im_get_reply_branch",
        br#"{"req_id":"same","channel_id":"c1","root_message_id":"root","reply_first_level_id":"first"}"#,
        9,
    )
    .unwrap();
    assert_eq!(snapshot.mode, ReplyProjectionMode::Snapshot);
    assert_eq!(append.mode, ReplyProjectionMode::Append);
    assert_eq!(snapshot.root_hint, "root");
    assert_eq!(append.root_hint, "root");
    assert!(
        ReplyProjectionRequest::from_command("im_get_replies", br#"{"reply_id":"root"}"#, 10)
            .is_none()
    );
}

#[test]
fn old_revision_is_rejected_and_snapshot_replaces_seen_ids() {
    let mut revisions = std::collections::HashMap::new();
    let mut seen_ids = std::collections::HashMap::new();

    let mut latest = FlatReplyProjection {
        root_message_id: "root".into(),
        revision: 9,
        reply_ids: vec!["r2".into()],
        ..FlatReplyProjection::empty()
    };
    assert!(accept_projection(
        &mut latest,
        &mut revisions,
        &mut seen_ids
    ));

    let mut old = FlatReplyProjection {
        root_message_id: "root".into(),
        revision: 8,
        reply_ids: vec!["r1".into()],
        ..FlatReplyProjection::empty()
    };
    assert!(!accept_projection(&mut old, &mut revisions, &mut seen_ids));
    assert_eq!(
        seen_ids["root"],
        std::collections::HashSet::from(["r2".into()])
    );
}

#[test]
fn append_deduplicates_across_pages_and_preserves_new_order() {
    let mut revisions = std::collections::HashMap::from([("root".into(), 2)]);
    let mut seen_ids = std::collections::HashMap::from([(
        "root".into(),
        std::collections::HashSet::from(["r1".into()]),
    )]);
    let mut append = FlatReplyProjection {
        root_message_id: "root".into(),
        projection_mode: ReplyProjectionMode::Append,
        revision: 3,
        reply_ids: vec!["r1".into(), "r2".into(), "r3".into()],
        nodes: vec![
            serde_json::json!({"msgId":"r1"}),
            serde_json::json!({"msgId":"r2"}),
            serde_json::json!({"msgId":"r3"}),
        ],
        ..FlatReplyProjection::empty()
    };
    assert!(accept_projection(
        &mut append,
        &mut revisions,
        &mut seen_ids
    ));
    assert_eq!(append.reply_ids, vec!["r2", "r3"]);
    assert_eq!(append.nodes[0]["msgId"], "r2");
    assert_eq!(append.nodes[1]["msgId"], "r3");
}
