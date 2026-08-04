//! Mobile UC10 / G-10a canonical 合同：UI 只给 anchor post id，时间边界来自 Go 权威回包。

use helix_im::older_context::{
    build_post_context_body, parse_request, LoadOlderState, RoundDecision,
};
use helix_im::state::ChannelId;
use serde_json::json;

const CH: &str = "ch00000000000000000000000a";
const OTHER_CH: &str = "ch00000000000000000000000b";

fn state(before: u32) -> LoadOlderState {
    LoadOlderState::new(
        ChannelId::from_str(CH).unwrap(),
        "anchor".to_string(),
        before,
    )
}

fn post(id: &str, tmp: &str, create_at: i64, channel_id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "temporaryId": tmp,
        "createAt": create_at,
        "channelId": channel_id,
    })
}

#[test]
/// 验证移动端上拉请求接受默认值与完整合法页大小区间。
fn canonical_request_accepts_only_channel_anchor_and_optional_before() {
    let default = serde_json::to_vec(&json!({
        "channel_id": CH,
        "anchor_post_id": "anchor"
    }))
    .unwrap();
    let state = parse_request(&default).expect("canonical default before");
    assert_eq!(state.target(), 40);
    assert_eq!(state.pivot_id(), "anchor");
    assert_eq!(state.request_id(), None);

    for before in [1, 60] {
        let payload = serde_json::to_vec(&json!({
            "channel_id": CH,
            "anchor_post_id": "anchor",
            "before": before
        }))
        .unwrap();
        assert_eq!(parse_request(&payload).unwrap().target(), before);
    }
}

#[test]
/// 验证移动端上拉请求拒绝越界页大小与旧版字段。
fn canonical_request_rejects_out_of_range_or_legacy_unknown_fields() {
    for before in [0, 61] {
        let payload = serde_json::to_vec(&json!({
            "channel_id": CH,
            "anchor_post_id": "anchor",
            "before": before
        }))
        .unwrap();
        assert!(parse_request(&payload).is_err());
    }
    let wrong_type = serde_json::to_vec(&json!({
        "channel_id": CH,
        "anchor_post_id": "anchor",
        "before": "20"
    }))
    .unwrap();
    assert!(parse_request(&wrong_type).is_err());

    for legacy in ["anchor_create_at", "req_id", "limit", "unknown"] {
        let mut payload = json!({"channel_id": CH, "anchor_post_id": "anchor"});
        payload[legacy] = json!(1);
        assert!(parse_request(&serde_json::to_vec(&payload).unwrap()).is_err());
    }
}

#[test]
fn post_context_body_uses_canonical_before_not_fixed_batch() {
    let state = state(37);
    let (path, body) = build_post_context_body(&state);
    assert_eq!(path, "posts/postContext");
    assert_eq!(body, json!({"postId": "anchor", "before": 37}));
}

#[test]
fn first_reply_derives_boundary_from_anchor_and_excludes_non_older_rows() {
    let mut state = state(20);
    let decision = state.ingest_round(&[
        post("wrong-channel", "t-wrong", 800, OTHER_CH),
        post("older", "t-old", 900, CH),
        post("anchor", "t-anchor", 1_000, CH),
        post("after", "t-after", 1_100, CH),
    ]);
    assert_eq!(decision, RoundDecision::Continue);
    assert_eq!(state.anchor_create_at(), 1_000);
    assert_eq!(state.older_count(), 1);
}

#[test]
fn missing_anchor_or_bad_post_fields_publish_no_history() {
    let mut missing_anchor = state(20);
    assert_eq!(
        missing_anchor.ingest_round(&[post("older", "t-old", 900, CH)]),
        RoundDecision::Done
    );
    assert_eq!(missing_anchor.older_count(), 0);
    assert!(!missing_anchor.has_more());

    let mut bad_fields = state(20);
    assert_eq!(
        bad_fields.ingest_round(&[
            post("anchor", "t-anchor", 1_000, CH),
            post("good-before-bad", "t-good", 900, CH),
            json!({"id":"older", "temporaryId":"t-old", "channelId":CH}),
        ]),
        RoundDecision::Done
    );
    assert_eq!(bad_fields.older_count(), 0);
    assert!(!bad_fields.has_more());
}
