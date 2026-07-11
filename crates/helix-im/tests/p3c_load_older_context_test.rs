//! 回归锚: D3（上拉 postContext 翻页投影 — NEED_HELIX_FIX_PULL_REFRESH）
//!
//! `im_load_older_context` 多轮 postContext 编排的**纯逻辑**性质测试：
//! - 严格更早过滤（`createAt < anchor_create_at`，对齐 legacy `fetch_older_posts_by_context`）
//! - 跨批 `temporaryId` 去重（锚点 / [T0,Tk) 重叠批不重复 prepend）
//! - pivot 推进（本批最旧、带非空 server `id` 的一条；服务端 self 消息带真 id 越过本地 id='' 墙）
//! - 续/停判定（凑够 target / 服务端无更早 / pivot 无法前进 / 轮数耗尽 → 停）
//! - hasMore 推断（停因「无更早 / pivot 无法前进 / 服务端空批」→ false；停因「凑够 target」→ true）
//! - emit 投影 `im:messages:older_loaded{channelId, messages[], hasMore}`（透传 wire Post camelCase）

use helix_im::older_context::{
    build_post_context_body, emit_older_loaded, parse_request, post_context_http_tracked,
    LoadOlderState, RoundDecision,
};
use helix_im::state::ChannelId;
use serde_json::json;

const CH: &str = "ch00000000000000000000000a";

/// 构造一条 wire Post（postContext 响应数组元素，camelCase 真源）。
fn post(id: &str, temporary_id: &str, create_at: i64) -> serde_json::Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "createAt": create_at,
        "channelId": CH,
        "message": format!("msg-{temporary_id}"),
    })
}

fn state(anchor_id: &str, anchor_create_at: i64, target: u32) -> LoadOlderState {
    LoadOlderState::new(
        ChannelId::from_str(CH).unwrap(),
        anchor_id.to_string(),
        anchor_create_at,
        target,
    )
}

// ── parse_request ──────────────────────────────────────────────────────────

#[test]
fn parse_request_extracts_anchor_and_clamps_target() {
    let payload = serde_json::to_vec(&json!({
        "channel_id": CH,
        "anchor_post_id": "p-anchor",
        "anchor_create_at": 1_000_i64,
        "req_id": "mrc-g10a:req-1",
        "limit": 20
    }))
    .unwrap();
    let st = parse_request(&payload).expect("parse ok");
    assert_eq!(st.channel_id().as_str(), CH);
    assert_eq!(st.anchor_create_at(), 1_000);
    assert_eq!(st.target(), 20);
    assert_eq!(st.pivot_id(), "p-anchor");
    assert_eq!(st.request_id(), Some("mrc-g10a:req-1"));
}

#[test]
fn parse_request_clamps_limit_to_1_100() {
    let big = serde_json::to_vec(&json!({
        "channel_id": CH, "anchor_post_id": "p", "anchor_create_at": 1, "limit": 9999
    }))
    .unwrap();
    assert_eq!(parse_request(&big).unwrap().target(), 100);
    // limit 0 / 缺省 → clamp 到 ≥1。
    let zero = serde_json::to_vec(&json!({
        "channel_id": CH, "anchor_post_id": "p", "anchor_create_at": 1, "limit": 0
    }))
    .unwrap();
    assert_eq!(parse_request(&zero).unwrap().target(), 1);
    let missing = serde_json::to_vec(&json!({
        "channel_id": CH, "anchor_post_id": "p", "anchor_create_at": 1
    }))
    .unwrap();
    assert_eq!(parse_request(&missing).unwrap().target(), 20);
}

#[test]
fn parse_request_rejects_missing_or_bad_fields() {
    // 缺 channel_id。
    let p1 = serde_json::to_vec(&json!({"anchor_post_id":"p","anchor_create_at":1})).unwrap();
    assert!(parse_request(&p1).is_err());
    // 缺 anchor_post_id。
    let p2 = serde_json::to_vec(&json!({"channel_id":CH,"anchor_create_at":1})).unwrap();
    assert!(parse_request(&p2).is_err());
    // 缺 anchor_create_at。
    let p3 = serde_json::to_vec(&json!({"channel_id":CH,"anchor_post_id":"p"})).unwrap();
    assert!(parse_request(&p3).is_err());
    // 非法 channel_id（长度不符 Id26）。
    let p4 =
        serde_json::to_vec(&json!({"channel_id":"bad","anchor_post_id":"p","anchor_create_at":1}))
            .unwrap();
    assert!(parse_request(&p4).is_err());
    // 坏 JSON 不 panic。
    assert!(parse_request(b"not json{").is_err());
}

// ── build_post_context_body ────────────────────────────────────────────────

#[test]
fn build_post_context_body_uses_pivot_and_batch_50() {
    let st = state("p-anchor", 1_000, 20);
    let (path, body) = build_post_context_body(&st);
    assert_eq!(path, "posts/postContext");
    // 真源 body {postId, before:int}（http.rs:96 batch=50）。
    assert_eq!(body["postId"], "p-anchor");
    assert_eq!(body["before"], 50);
}

#[test]
fn post_context_http_carries_current_run_track_id() {
    let st = parse_request(
        &serde_json::to_vec(&json!({
            "channel_id": CH,
            "anchor_post_id": "p-anchor",
            "anchor_create_at": 1_000_i64,
            "req_id": "mrc-g10a:req-1"
        }))
        .unwrap(),
    )
    .unwrap();
    let (_, body) = build_post_context_body(&st);
    let effect = post_context_http_tracked(
        "http://127.0.0.1:8066/api/cses",
        &body,
        helix_core::Correlation::from_raw(7),
        Some("conn-1"),
        st.request_id(),
    );
    let helix_core::effect::Effect::Http { req, .. } = effect else {
        panic!("expected Http")
    };
    assert!(req
        .headers
        .iter()
        .any(|(name, value)| name == "Cses-Track-Id" && value == "mrc-g10a:req-1"));
}

// ── ingest_round：严格更早 + 去重 + pivot + 续停判定 ──────────────────────────

#[test]
fn ingest_round_keeps_only_strictly_older_and_dedups() {
    let mut st = state("p-anchor", 1_000, 20);
    // 锚点同条(=1000) + 更新条(>1000) 不收；更早 2 条收。
    let rows = vec![
        post("id-anchor", "t-anchor", 1_000), // 同毫秒，非严格更早 → 砍
        post("id-newer", "t-newer", 1_100),   // 更新 → 砍
        post("id-old1", "t-old1", 900),       // 更早 → 收
        post("id-old2", "t-old2", 800),       // 更早 → 收
    ];
    let decision = st.ingest_round(&rows);
    assert_eq!(st.older_count(), 2);
    // 还没凑够 target(20) 且 pivot 能前进 → 继续。
    assert!(matches!(decision, RoundDecision::Continue));

    // 重叠批：t-old1 重复 + 新更早 t-old3 → 只 t-old3 进。
    let rows2 = vec![
        post("id-old1", "t-old1", 900),
        post("id-old3", "t-old3", 700),
    ];
    st.ingest_round(&rows2);
    assert_eq!(st.older_count(), 3); // t-old1 去重，不二次计入
}

#[test]
fn ingest_round_stops_when_target_reached_hasmore_true() {
    let mut st = state("p-anchor", 1_000, 2);
    let rows = vec![
        post("id-old1", "t-old1", 900),
        post("id-old2", "t-old2", 800),
        post("id-old3", "t-old3", 700),
    ];
    let decision = st.ingest_round(&rows);
    // 收够 target(2) 即停，且服务端还有更早（id-old3 没全用上）→ hasMore=true。
    assert!(matches!(decision, RoundDecision::Done));
    assert!(st.has_more());
    // truncate 到 target。
    assert_eq!(st.older_count(), 2);
}

#[test]
fn ingest_round_stops_on_empty_batch_hasmore_false() {
    let mut st = state("p-anchor", 1_000, 20);
    let decision = st.ingest_round(&[]);
    assert!(matches!(decision, RoundDecision::Done));
    assert!(!st.has_more()); // 服务端无更早
    assert_eq!(st.older_count(), 0);
}

#[test]
fn ingest_round_stops_when_no_newer_oldest_hasmore_false() {
    let mut st = state("p-anchor", 1_000, 20);
    // 第一批最旧 = 800。
    st.ingest_round(&[post("id-a", "t-a", 900), post("id-b", "t-b", 800)]);
    // 第二批最旧 ≥ 上批最旧（没翻出更早）→ 服务端耗尽 → 停，hasMore=false。
    let decision = st.ingest_round(&[post("id-c", "t-c", 850), post("id-d", "t-d", 820)]);
    assert!(matches!(decision, RoundDecision::Done));
    assert!(!st.has_more());
}

#[test]
fn ingest_round_stops_when_pivot_cannot_advance() {
    // 全批 server id 为空（全是本地未带 id 的 self 消息）→ next_pivot 无 → 无法前进 → 停。
    let mut st = state("p-anchor", 1_000, 20);
    let rows = vec![
        json!({"id":"", "temporaryId":"t-x", "createAt":900, "channelId":CH}),
        json!({"id":"", "temporaryId":"t-y", "createAt":800, "channelId":CH}),
    ];
    let decision = st.ingest_round(&rows);
    assert!(matches!(decision, RoundDecision::Done));
    assert!(!st.has_more());
    // 更早消息仍收集（即便 pivot 不能再前进，本批更早部分要回灌）。
    assert_eq!(st.older_count(), 2);
}

#[test]
fn ingest_round_exhausts_rounds_budget() {
    // 每轮翻出 1 条更早 + pivot 能前进，但 target 很大 → 轮数预算（8）耗尽即停。
    let mut st = state("p-anchor", 10_000, 1_000);
    let mut last = RoundDecision::Continue;
    for r in 0..9 {
        let ts = 9_000 - r * 100; // 单调更早，保证每轮翻出更早
        last = st.ingest_round(&[post(&format!("id-{r}"), &format!("t-{r}"), ts as i64)]);
    }
    // 第 9 轮（index 8）超出预算 → 停。
    assert!(matches!(last, RoundDecision::Done));
    // 仍有更早（预算耗尽而非服务端耗尽）→ hasMore=true。
    assert!(st.has_more());
}

#[test]
fn pivot_advances_to_oldest_with_server_id() {
    let mut st = state("p-anchor", 1_000, 20);
    // 最旧带 id 的是 id-old2(800)。
    st.ingest_round(&[
        post("id-old1", "t-old1", 900),
        post("id-old2", "t-old2", 800),
    ]);
    assert_eq!(st.pivot_id(), "id-old2");
}

// ── emit_older_loaded ───────────────────────────────────────────────────────

#[test]
fn emit_older_loaded_shape_matches_contract() {
    let mut st = state("p-anchor", 1_000, 20);
    st.ingest_round(&[
        post("id-old1", "t-old1", 900),
        post("id-old2", "t-old2", 800),
    ]);
    let effect = emit_older_loaded(&st);
    let v = effect_to_json(effect);
    assert_eq!(v["event"], "im:messages:older_loaded");
    assert_eq!(v["data"]["channelId"], CH);
    // 升序回灌（前端 prepend）：800 在 900 前。
    assert_eq!(v["data"]["messages"][0]["createAt"], 800);
    assert_eq!(v["data"]["messages"][1]["createAt"], 900);
    // 透传 camelCase 字段（HX-C005 不改业务数据）。
    assert_eq!(v["data"]["messages"][0]["temporaryId"], "t-old2");
    assert_eq!(v["data"]["hasMore"], false);
}

#[test]
fn emit_older_loaded_empty_yields_empty_messages_no_panic() {
    let st = state("p-anchor", 1_000, 20);
    let v = effect_to_json(emit_older_loaded(&st));
    assert_eq!(v["data"]["messages"], json!([]));
    assert_eq!(v["data"]["hasMore"], false);
}

fn effect_to_json(effect: helix_core::effect::Effect) -> serde_json::Value {
    match effect {
        helix_core::effect::Effect::Emit { event } => {
            serde_json::from_slice(event.0.as_ref()).unwrap()
        }
        _ => panic!("expected Emit"),
    }
}
