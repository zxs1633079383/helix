//! UC-10 todo 装配单测（sibling 外提，守 todo.rs ≤300 + 关注点拆分）。
//!
//! 回归锚: UC-10 —— about-me 收集 / queryTodoList body / 响应装配 emit im:todo:updated。

use super::*;

fn emit_json(e: Effect) -> Value {
    match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
        _ => panic!("expected Emit"),
    }
}

// ── collect_about_me_ids（mention + urgent 两源拼接，真源 channel.rs:142-148）──────────

#[test]
fn collect_merges_mention_and_urgent_in_order() {
    let data = json!({
        "id": "ch1",
        "mentionList": ["p1", "p2"],
        "urgentPostList": ["p3"]
    });
    assert_eq!(collect_about_me_ids(&data), vec!["p1", "p2", "p3"]);
}

#[test]
fn collect_reads_nested_channel_object() {
    let data = json!({ "channel": { "mentionList": ["m1"], "urgentPostList": ["u1"] } });
    assert_eq!(collect_about_me_ids(&data), vec!["m1", "u1"]);
}

#[test]
fn collect_missing_lists_yields_empty() {
    assert!(collect_about_me_ids(&json!({ "id": "ch1" })).is_empty());
}

#[test]
fn collect_skips_non_string_elements() {
    let data = json!({ "mentionList": ["p1", 42, null, "p2"] });
    assert_eq!(collect_about_me_ids(&data), vec!["p1", "p2"]);
}

// ── query_todo_body（真源 partials/1 §18 `{postIds:[]string}`）────────────────────────

#[test]
fn query_body_is_post_ids_array() {
    let body = query_todo_body(&["p1".into(), "p2".into()]);
    assert_eq!(body, json!({ "postIds": ["p1", "p2"] }));
}

// ── emit_todo_updated 装配（真源 channel.rs:252-296 + 346-349）────────────────────────

#[test]
fn emit_assembles_items_with_id_type_candel() {
    let raw = serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": { "channels": [
            { "id": "chA", "teamId": "t1", "posts": [
                { "id": "post1", "messageType": "mention" },
                { "id": "post2", "messageType": "urgent" }
            ]}
        ]}
    }))
    .unwrap();

    let v = emit_json(emit_todo_updated(&raw));
    assert_eq!(v["event"], "im:todo:updated");
    let items = v["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2);

    // item1：mention → id "post1_mention"、canDel=true、channel/post 透传。
    assert_eq!(items[0]["id"], "post1_mention");
    assert_eq!(items[0]["type"], "mention");
    assert_eq!(items[0]["canDel"], true);
    assert_eq!(items[0]["channel"]["id"], "chA");
    assert_eq!(items[0]["post"]["id"], "post1");

    // item2：urgent → canDel=false（仅 mention 可删）。
    assert_eq!(items[1]["id"], "post2_urgent");
    assert_eq!(items[1]["canDel"], false);
}

#[test]
fn emit_accepts_numeric_status_200() {
    let raw = serde_json::to_vec(&json!({
        "status": 200,
        "data": { "channels": [{ "id": "c", "posts": [{ "id": "p", "messageType": "mention" }] }] }
    }))
    .unwrap();
    let v = emit_json(emit_todo_updated(&raw));
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 1);
}

#[test]
fn emit_non_success_yields_empty_items_still_emits() {
    let raw = serde_json::to_vec(&json!({ "status": "FAIL", "data": { "channels": [] } })).unwrap();
    let v = emit_json(emit_todo_updated(&raw));
    assert_eq!(v["event"], "im:todo:updated");
    assert!(v["data"]["items"].as_array().unwrap().is_empty());
}

#[test]
fn emit_bad_json_degrades_to_empty_no_panic() {
    let v = emit_json(emit_todo_updated(b"not json at all"));
    assert!(v["data"]["items"].as_array().unwrap().is_empty());
}

#[test]
fn emit_missing_message_type_uses_empty_suffix() {
    let raw = serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": { "channels": [{ "id": "c", "posts": [{ "id": "post9" }] }] }
    }))
    .unwrap();
    let v = emit_json(emit_todo_updated(&raw));
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items[0]["id"], "post9_");
    assert_eq!(items[0]["canDel"], false);
}

#[test]
fn emit_payload_is_object_not_bare_array() {
    // 信封铁律：data 是 {items:[...]} object，禁裸数组（前端 Electron 多参兼容层防误判）。
    let raw =
        serde_json::to_vec(&json!({ "status": "SUCCESS", "data": { "channels": [] } })).unwrap();
    let v = emit_json(emit_todo_updated(&raw));
    assert!(v["data"].is_object());
    assert!(v["data"]["items"].is_array());
}
