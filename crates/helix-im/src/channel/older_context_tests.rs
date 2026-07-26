//! `older_context.rs` 内部单元测试（sibling，结构闸 ≤300 行外提）。
//!
//! 覆盖 sort_key 同毫秒确定化 + stop truncate + RoundDecision Debug/Eq 等内部分支，
//! 与 `tests/p3c_load_older_context_test.rs`（公共 API 性质）互补。

use super::*;
use serde_json::json;

const CH: &str = "ch00000000000000000000000a";

fn row(id: &str, tmp: &str, at: i64) -> Value {
    json!({ "id": id, "temporaryId": tmp, "createAt": at, "channelId": CH })
}

fn st() -> LoadOlderState {
    LoadOlderState::new(ChannelId::from_str(CH).unwrap(), "anchor".to_string(), 50)
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
// Proves stop keeps the canonical oldest-first subset before persistence.
fn stop_truncates_to_target_and_sorts_ascending() {
    let mut s = LoadOlderState::new(
        ChannelId::from_str(CH).unwrap(),
        "anchor".to_string(),
        2, // target=2
    );
    // 一轮收 4 条更早（乱序）→ stop 应排序升序 + truncate(2)。
    let rows = vec![
        row("anchor", "t-anchor", 10_000),
        row("i1", "t1", 900),
        row("i2", "t2", 700),
        row("i3", "t3", 800),
        row("i4", "t4", 600),
    ];
    let d = s.ingest_round(&rows);
    assert!(matches!(d, RoundDecision::Done)); // 凑够 target
    assert_eq!(s.older_count(), 2);
    // 最早两条（600, 700）升序。
    let older = s.older_rows();
    assert_eq!(older[0]["createAt"], 600);
    assert_eq!(older[1]["createAt"], 700);
}

#[test]
fn round_decision_is_debug_and_eq() {
    assert_eq!(RoundDecision::Continue, RoundDecision::Continue);
    assert_ne!(RoundDecision::Continue, RoundDecision::Done);
    assert_eq!(format!("{:?}", RoundDecision::Done), "Done");
}

#[test]
fn build_body_uses_target_and_path() {
    let s = st();
    let (path, body) = build_post_context_body(&s);
    assert_eq!(path, "posts/postContext");
    assert_eq!(body["before"], 50);
    assert_eq!(body["postId"], "anchor");
}

#[test]
// Proves transport correlation is accepted without leaking into the Go body.
fn request_id_is_header_only_sidecar() {
    let state = parse_request(
        serde_json::to_vec(&json!({
            "channel_id": CH,
            "anchor_post_id": "anchor",
            "before": 20,
            "req_id": "mrc-g10a-1",
        }))
        .expect("serialize request")
        .as_slice(),
    )
    .expect("parse correlated request");

    assert_eq!(state.request_id(), Some("mrc-g10a-1"));
    let (_, body) = build_post_context_body(&state);
    assert!(body.get("req_id").is_none());
}

#[test]
// Proves malformed host correlation fails closed before any HTTP Effect can exist.
fn request_id_rejects_empty_or_non_string_values() {
    for req_id in [json!(""), json!(7)] {
        let error = parse_request(
            serde_json::to_vec(&json!({
                "channel_id": CH,
                "anchor_post_id": "anchor",
                "req_id": req_id,
            }))
            .expect("serialize malformed request")
            .as_slice(),
        )
        .expect_err("malformed req_id must fail");
        assert!(error.to_string().contains("req_id 必须为非空字符串"));
    }
}

#[test]
fn rows_missing_temporary_id_are_not_collected() {
    let mut s = st();
    // 更早但缺 temporaryId → 整轮 fail-closed，不留下部分历史。
    s.ingest_round(&[
        row("anchor", "t-anchor", 10_000),
        json!({"id":"i1","createAt":900,"channelId":CH}),
    ]);
    assert_eq!(s.older_count(), 0);
}
