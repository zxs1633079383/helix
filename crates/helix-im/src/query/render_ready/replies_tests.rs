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
    assert_eq!(extract_reply_ids(&body), vec!["root1", "r1", "r2"]);
}

#[test]
fn getreplybranch_body_data_and_list_and_toplevel_array() {
    // getReplyBranch 形态：HasMorePage[Post]（{data:[]} / {list:[]} / 顶层数组）。
    let data_form = serde_json::json!({"data": [{"id": "a"}, {"id": "b"}]});
    assert_eq!(extract_reply_ids(&data_form), vec!["a", "b"]);

    let list_form = serde_json::json!({"list": [{"id": "c"}]});
    assert_eq!(extract_reply_ids(&list_form), vec!["c"]);

    let top_array = serde_json::json!([{"id": "x"}, {"id": "y"}, {"id": "x"}]);
    assert_eq!(extract_reply_ids(&top_array), vec!["x", "y"]);
}

#[test]
fn malformed_or_error_body_yields_empty() {
    // 失败回灌 {req_id, error} 无 reply 形态 / 非对象非数组 → 空（前端 reject·清抽屉）。
    assert!(extract_reply_ids(&serde_json::json!({"error": "boom"})).is_empty());
    assert!(extract_reply_ids(&serde_json::Value::Null).is_empty());
    assert!(extract_reply_ids(&serde_json::json!("plain string")).is_empty());
    assert!(extract_reply_ids(&serde_json::json!(42)).is_empty());
}

#[test]
fn health_body_status_not_treated_as_reply() {
    // UC-12.1 健康探针 body {status:"OK"} 共用 im:read:result·无 reply 形态 → replyIds 空（不误塞）。
    assert!(extract_reply_ids(&serde_json::json!({"status": "OK"})).is_empty());
}

#[test]
fn emit_channel_replies_shape_camelcase() {
    let e = emit_channel_replies("rq-1", vec!["p1".into(), "p2".into()]);
    let v = emit_json(e);
    assert_eq!(v["event"], "im:channel:replies");
    assert_eq!(v["data"]["reqId"], "rq-1");
    assert_eq!(v["data"]["replyIds"][0], "p1");
    assert_eq!(v["data"]["replyIds"][1], "p2");
}

#[test]
fn emit_channel_replies_empty_clears_drawer() {
    // 失败/空 → replyIds:[]（壳 applyChannelReplies set([]) 清抽屉·reject 语义）。
    let v = emit_json(emit_channel_replies("rq-2", Vec::new()));
    assert_eq!(v["data"]["reqId"], "rq-2");
    assert!(v["data"]["replyIds"].as_array().unwrap().is_empty());
}
