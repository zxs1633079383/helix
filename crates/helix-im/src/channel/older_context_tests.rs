//! `older_context.rs` 内部单元测试（sibling，结构闸 ≤300 行外提）。
//!
//! 覆盖 sort_key 同毫秒确定化 + stop truncate + RoundDecision Debug/Eq 等内部分支，
//! 与 `tests/p3c_load_older_context_test.rs`（公共 API 性质）互补。

use super::*;
use helix_core::Effect;

const CH: &str = "ch00000000000000000000000a";

fn row(id: &str, tmp: &str, at: i64) -> Value {
    json!({ "id": id, "temporaryId": tmp, "createAt": at, "channelId": CH })
}

fn st() -> LoadOlderState {
    LoadOlderState::new(
        ChannelId::from_str(CH).unwrap(),
        "anchor".to_string(),
        10_000,
        50,
    )
}

#[test]
fn sort_key_orders_by_createat_then_temporary_id() {
    // 同毫秒（createAt 相同）→ temporaryId 确定化（HX-C010 有序 Effect）。
    assert!(sort_key(&row("i1", "t-b", 100)) > sort_key(&row("i2", "t-a", 100)));
    // createAt 主序压过 temporaryId。
    assert!(sort_key(&row("i1", "t-a", 200)) > sort_key(&row("i2", "t-z", 100)));
    // 缺字段 → 默认 (0, "")，不 panic。
    assert_eq!(sort_key(&json!({})), (0, String::new()));
}

#[test]
fn stop_truncates_to_target_and_sorts_ascending() {
    let mut s = LoadOlderState::new(
        ChannelId::from_str(CH).unwrap(),
        "anchor".to_string(),
        10_000,
        2, // target=2
    );
    // 一轮收 4 条更早（乱序）→ stop 应排序升序 + truncate(2)。
    let rows = vec![
        row("i1", "t1", 900),
        row("i2", "t2", 700),
        row("i3", "t3", 800),
        row("i4", "t4", 600),
    ];
    let d = s.ingest_round(&rows);
    assert!(matches!(d, RoundDecision::Done)); // 凑够 target
    assert_eq!(s.older_count(), 2);
    // 最早两条（600, 700）升序。
    let e = emit_older_loaded(&s);
    let v = match e {
        Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).unwrap(),
        _ => panic!("emit"),
    };
    assert_eq!(v["data"]["messages"][0]["createAt"], 600);
    assert_eq!(v["data"]["messages"][1]["createAt"], 700);
}

#[test]
fn round_decision_is_debug_and_eq() {
    assert_eq!(RoundDecision::Continue, RoundDecision::Continue);
    assert_ne!(RoundDecision::Continue, RoundDecision::Done);
    assert_eq!(format!("{:?}", RoundDecision::Done), "Done");
}

#[test]
fn build_body_constant_batch_and_path() {
    let s = st();
    let (path, body) = build_post_context_body(&s);
    assert_eq!(path, "posts/postContext");
    assert_eq!(body["before"], 50);
    assert_eq!(body["postId"], "anchor");
}

#[test]
fn rows_missing_temporary_id_are_not_collected() {
    let mut s = st();
    // 更早但缺 temporaryId → 不进 older（去重键空，避免空键碰撞）。
    s.ingest_round(&[json!({"id":"i1","createAt":900,"channelId":CH})]);
    assert_eq!(s.older_count(), 0);
}
