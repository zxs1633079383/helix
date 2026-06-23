//! UC-10 通知/待办装配链 e2e 回放测试（hello → increment(mention/urgent) → end → queryTodoList
//! → 装配 emit im:todo:updated）。
//!
//! 回归锚: UC-10 —— hello 收尾触发 queryTodoList + 待办投影 emit + 会话边界清缓冲。
//!
//! ## 被测不变量（行为真源 cses-client handlers/channel.rs:142-355）
//! 1. `increment_channel` 帧的 `mentionList + urgentPostList` 累入会话缓冲。
//! 2. global `increment_channel_end` 收尾 → build `posts/queryTodoList {postIds}` HTTP + 注册
//!    `CorrelationContext::TodoQuery`，**读完即清空**缓冲。
//! 3. queryTodoList HTTP 回报 → 解 `data.channels[].posts[]` 装配 emit `im:todo:updated{items}`。
//!
//! ## 可证伪性（HX-C011）
//! - `assembles_todo_from_increment_mentions` 依赖 increment_channel.rs 的 `collect_about_me_ids`
//!   累积 + increment_channel_end.rs 的 `trigger_todo_query`：**删任一** → 无 queryTodoList Http /
//!   无 TodoQuery 注册 → PortReply 不 emit → FAIL（正是 UC-10 缺口现状：待办永不装配）。
//! - `empty_about_me_skips_todo_query` 反向：无 mention/urgent → **不应**发 queryTodoList（证伪
//!   「无条件每次 hello 都拉」的过度触发）。

mod common;
use common::*;

use helix_core::effect::TransportId;
use helix_core::{Effect, EffectSink, Tick};
use helix_im::state::Seq;
use helix_im::sync_session::IncrementChannel;

// ─── 局部 helper ──────────────────────────────────────────────────────────────

/// 构造一条 increment_channel 帧 data（raw），携 mentionList/urgentPostList。
fn inc_with_lists(channel_num: u64, mentions: &[&str], urgents: &[&str]) -> IncrementChannel {
    use serde_json::json;
    let raw = json!({
        "id": cid(channel_num),
        "mentionList": mentions,
        "urgentPostList": urgents,
    });
    IncrementChannel {
        channel_id: channel_id(channel_num),
        last_event_seq: Seq(1),
        need_sync: false, // need_sync=false → 不混入 resync，隔离断言 queryTodoList
        raw: bytes::Bytes::from(serde_json::to_vec(&raw).unwrap()),
    }
}

/// 取本批次 `posts/queryTodoList` 的 (corr, body)（按 URL 后缀精确匹配，鲁棒不硬编码 corr）。
fn query_todo_http(sink: &EffectSink) -> Option<(u64, serde_json::Value)> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Http { corr, req } if req.url.ends_with("/posts/queryTodoList") => {
            let body = req
                .body
                .as_ref()
                .and_then(|b| serde_json::from_slice(b.as_ref()).ok())
                .unwrap_or(serde_json::Value::Null);
            Some((corr.raw(), body))
        }
        _ => None,
    })
}

/// 取本批次第一条 `im:todo:updated` emit payload。
fn todo_emit(sink: &EffectSink) -> Option<serde_json::Value> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => {
            let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (v.get("event").and_then(|x| x.as_str()) == Some("im:todo:updated")).then_some(v)
        }
        _ => None,
    })
}

/// 驱动到「hello + increment(channel, lists) + global end」之后，返回最后一批 sink 引用所在 Harness。
fn drive_to_increment_end(h: &mut Harness, inc: IncrementChannel) {
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_TODO"), 200);
    // 喂 increment_channel（携 mention/urgent）累入 about_me 缓冲。
    h.sink.clear();
    h.module.ingest_increment(&inc, &mut h.sink);
    // global increment end（None=全局收尾）→ 触发 queryTodoList。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
}

// ─── 不变量 1+2：mention/urgent 累积 → 收尾 build queryTodoList + 注册 corr ───────────

#[test]
fn assembles_todo_from_increment_mentions() {
    let mut h = Harness::new(&[]);
    drive_to_increment_end(
        &mut h,
        inc_with_lists(1, &["postaaa", "postbbb"], &["posturg"]),
    );

    // ① 收尾应发 queryTodoList，body = {postIds:[mention..., urgent...]}（顺序保真）。
    let (corr, body) = query_todo_http(&h.sink).expect("global end 应 build queryTodoList HTTP");
    assert_eq!(
        body["postIds"],
        serde_json::json!(["postaaa", "postbbb", "posturg"]),
        "postIds = mentionList ++ urgentPostList（顺序保真）"
    );

    // ② queryTodoList HTTP 回报（ADR-007 信封包裹的后端响应体）→ 装配 emit im:todo:updated。
    let raw_go = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": "SUCCESS",
            "data": { "channels": [
                { "id": cid(1), "teamId": "team1", "posts": [
                    { "id": "postaaa", "messageType": "mention" },
                    { "id": "posturg", "messageType": "urgent" }
                ]}
            ]}
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw_go)), 300);

    let emit = todo_emit(&h.sink).expect("queryTodoList 回报应 emit im:todo:updated");
    let items = emit["data"]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 2, "两 post 装配两 todo item");
    assert_eq!(items[0]["id"], "postaaa_mention");
    assert_eq!(items[0]["canDel"], true, "mention 可删");
    assert_eq!(items[0]["channel"]["id"], cid(1), "channel 透传");
    assert_eq!(items[0]["post"]["id"], "postaaa", "post 透传");
    assert_eq!(items[1]["id"], "posturg_urgent");
    assert_eq!(items[1]["canDel"], false, "urgent 不可删");
}

// ─── 不变量 2 续：读完即清空缓冲（第二次 end 不再重发 queryTodoList）────────────────────

#[test]
fn buffer_cleared_after_trigger_no_double_query() {
    let mut h = Harness::new(&[]);
    drive_to_increment_end(&mut h, inc_with_lists(1, &["postx"], &[]));
    assert!(
        query_todo_http(&h.sink).is_some(),
        "首次 end 应发 queryTodoList"
    );

    // 再来一次 global end（无新 increment）→ 缓冲已清 → 不应再发 queryTodoList。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert!(
        query_todo_http(&h.sink).is_none(),
        "缓冲读完即清，第二次 end 不得重发 queryTodoList（防残留重拉）"
    );
}

// ─── 反向证伪：无 mention/urgent → 不发 queryTodoList（防过度触发）────────────────────

#[test]
fn empty_about_me_skips_todo_query() {
    let mut h = Harness::new(&[]);
    drive_to_increment_end(&mut h, inc_with_lists(1, &[], &[]));
    assert!(
        query_todo_http(&h.sink).is_none(),
        "无 about-me（mention/urgent）→ 不应发 queryTodoList（非无条件每次 hello 都拉）"
    );
    assert!(
        todo_emit(&h.sink).is_none(),
        "无 queryTodoList → 无 im:todo:updated emit"
    );
}

// ─── 会话边界：hello reset 清缓冲（跨会话不残留）────────────────────────────────────

#[test]
fn hello_resets_about_me_buffer() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN_1"), 200);
    // 第一会话攒 mention，但不收尾（模拟会话被打断）。
    h.sink.clear();
    h.module
        .ingest_increment(&inc_with_lists(1, &["stale"], &[]), &mut h.sink);

    // 新 hello（重连）→ reset_increment_batch 清 about_me_post_ids。
    h.step(hello_inbound("CONN_2"), 300);
    // 新会话无新 increment → global end 不应发 queryTodoList（旧 "stale" 已被 hello reset 清掉）。
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    assert!(
        query_todo_http(&h.sink).is_none(),
        "新 hello reset 后旧会话 mention 残留应被清，不得跨会话重拉"
    );
}
