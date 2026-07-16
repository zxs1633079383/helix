//! A2 最近消息 local-first 回放：Tick → Effect → PortReply → DomainEvent。
//!
//! 回归锚: A2 —— coverage、远端回退、会话隔离与缓存顺序。

mod common;
use common::*;

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp, TransportId};
use helix_core::tick::AppCommand;
use helix_core::Tick;
use helix_im::query::LocalStoreMode;
use serde_json::{json, Value};

fn query_tick(channel: u64, limit: u32) -> Tick {
    Tick::Command(AppCommand::new(
        "im_query_messages_by_channel",
        serde_json::to_vec(&json!({
            "channel_id": cid(channel),
            "limit": limit,
        }))
        .unwrap(),
    ))
}

fn local_row(id: &str, temporary_id: &str, create_at: i64, text: &str) -> Value {
    json!({
        "id": id,
        "temporary_id": temporary_id,
        "channel_id": cid(1),
        "user_id": "remote-user",
        "type": "TEXT",
        "message": text,
        "create_at": create_at,
    })
}

fn optimistic_row(temporary_id: &str, create_at: i64, text: &str) -> Value {
    json!({
        "id": "",
        "temporary_id": temporary_id,
        "channel_id": cid(1),
        "user_id": "self-user",
        "type": "TEXT",
        "message": text,
        "create_at": create_at,
        "send_status": "sending",
    })
}

fn remote_post(id: &str, temporary_id: &str, create_at: i64, text: &str) -> Value {
    json!({
        "id": id,
        "temporaryId": temporary_id,
        "channelId": cid(1),
        "userId": "remote-user",
        "type": "TEXT",
        "message": text,
        "simpleMessage": text,
        "viewers": ["all"],
        "mentions": [],
        "createAt": create_at,
    })
}

fn latest_posts_reply(posts: Vec<Value>) -> Bytes {
    let raw = Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": posts,
        }))
        .unwrap(),
    );
    enveloped(raw)
}

fn envelope_with_http_status(status: u16, posts: Vec<Value>) -> Bytes {
    let mut envelope: Value = serde_json::from_slice(latest_posts_reply(posts).as_ref()).unwrap();
    envelope["status"] = Value::from(status);
    Bytes::from(serde_json::to_vec(&envelope).unwrap())
}

fn corr_for_http(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("expected Effect::Http")
}

fn corr_for_persist(effects: &[Effect]) -> u64 {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("expected Effect::Persist")
}

fn emitted_values(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

#[test]
fn durable_complete_after_proven_cached_window_uses_local_only() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    let older = remote_post(&sid(1), "tmp-1", 100, "older");
    let newer = remote_post(&sid(2), "tmp-2", 200, "newer");

    assert!(matches!(
        harness.step(query_tick(1, 2), 1_000).as_slice(),
        [EffectKind::Persist(_)]
    ));
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    assert!(matches!(
        harness
            .step(
                port_reply_with_body(scan_corr, Bytes::from_static(b"[]")),
                1_100,
            )
            .as_slice(),
        [EffectKind::Http(_)]
    ));
    let http_corr = corr_for_http(harness.sink.as_slice());
    assert!(matches!(
        harness
            .step(
                port_reply_with_body(
                    http_corr,
                    latest_posts_reply(vec![older.clone(), newer.clone()]),
                ),
                1_200,
            )
            .as_slice(),
        [EffectKind::Persist(_)]
    ));
    let cache_corr = corr_for_persist(harness.sink.as_slice());
    assert_eq!(
        harness.step(port_reply_ok(cache_corr), 1_300),
        vec![EffectKind::Emit]
    );
    let events = emitted_values(harness.sink.as_slice());
    assert_eq!(events[0]["event"], "im:messages:query_result");
    assert_eq!(events[0]["data"]["messages"][0]["msgId"], sid(1));

    harness.step(query_tick(1, 2), 2_000);
    let second_scan_corr = corr_for_persist(harness.sink.as_slice());
    let cached_rows = Bytes::from(
        serde_json::to_vec(&json!([
            local_row(&sid(2), "tmp-2", 200, "newer"),
            local_row(&sid(1), "tmp-1", 100, "older"),
        ]))
        .unwrap(),
    );
    let effects = harness.step(port_reply_with_body(second_scan_corr, cached_rows), 2_100);
    assert_eq!(effects, vec![EffectKind::Emit]);
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })),
        "显式 coverage 对得上时必须零 HTTP"
    );
    let projection = &emitted_values(harness.sink.as_slice())[0];
    assert_eq!(projection["data"]["messages"][0]["msgId"], sid(1));
    assert_eq!(projection["data"]["messages"][1]["msgId"], sid(2));
}

#[test]
fn durable_partial_merges_remote_over_local_deduped_and_ordered() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    harness.step(query_tick(1, 25), 1_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    let local = Bytes::from(
        serde_json::to_vec(&json!([
            local_row(&sid(1), "tmp-dup", 100, "stale-local"),
            local_row(&sid(30), "tmp-local", 50, "local-only"),
        ]))
        .unwrap(),
    );
    assert!(matches!(
        harness
            .step(port_reply_with_body(scan_corr, local), 1_100)
            .as_slice(),
        [EffectKind::Http(_)]
    ));
    let http_corr = corr_for_http(harness.sink.as_slice());
    let remote = (1..=20)
        .map(|index| {
            let temporary_id = if index == 1 {
                "tmp-dup".to_string()
            } else {
                format!("tmp-{index}")
            };
            remote_post(
                &sid(index),
                &temporary_id,
                100 + index as i64,
                if index == 1 { "remote-wins" } else { "remote" },
            )
        })
        .collect();
    harness.step(
        port_reply_with_body(http_corr, latest_posts_reply(remote)),
        1_200,
    );
    let cache_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(port_reply_ok(cache_corr), 1_300);

    let projection = &emitted_values(harness.sink.as_slice())[0];
    let messages = projection["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 21, "同 server id/temporaryId 必须去重");
    assert_eq!(
        messages[0]["msgId"],
        sid(30),
        "旧本地行只能补在权威窗口之前"
    );
    assert_eq!(messages[1]["msgId"], sid(1));
    assert_eq!(messages[1]["text"], "remote-wins");
    assert_eq!(messages[20]["msgId"], sid(20));
}

#[test]
fn session_miss_caches_before_emitting_result() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Session);
    harness.step(query_tick(1, 20), 1_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(scan_corr, Bytes::from_static(b"[]")),
        1_100,
    );
    let http_corr = corr_for_http(harness.sink.as_slice());
    let effects = harness.step(
        port_reply_with_body(
            http_corr,
            latest_posts_reply(vec![remote_post(&sid(1), "tmp-1", 100, "remote")]),
        ),
        1_200,
    );
    assert!(matches!(effects.as_slice(), [EffectKind::Persist(_)]));
    assert!(harness.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::Persist { ops, .. }
            if ops.iter().any(|op| matches!(op, StorageOp::BatchUpsert(spec)
                if spec.table == "message")))
    }));
    assert!(emitted_values(harness.sink.as_slice()).is_empty());

    let cache_corr = corr_for_persist(harness.sink.as_slice());
    assert_eq!(
        harness.step(port_reply_ok(cache_corr), 1_300),
        vec![EffectKind::Emit]
    );
    let events = emitted_values(harness.sink.as_slice());
    assert_eq!(events[0]["event"], "im:messages:query_result");
    assert_eq!(events[0]["data"]["messages"][0]["msgId"], sid(1));
}

#[test]
fn remote_quick_reply_is_merged_into_render_ready_props() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Session);
    harness.step(query_tick(1, 20), 1_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(scan_corr, Bytes::from_static(b"[]")),
        1_100,
    );
    let http_corr = corr_for_http(harness.sink.as_slice());
    let mut post = remote_post(&sid(1), "tmp-1", 100, "reacted");
    post["props"] = json!({"channel_event_seq": 2});
    post["quickReply"] = json!([{"emoji": ":thumbsup:", "userIds": ["444"]}]);
    harness.step(
        port_reply_with_body(http_corr, latest_posts_reply(vec![post])),
        1_200,
    );
    let cache_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(port_reply_ok(cache_corr), 1_300);

    let projection = &emitted_values(harness.sink.as_slice())[0];
    let row = &projection["data"]["messages"][0];
    assert_eq!(row["props"]["channel_event_seq"], 2);
    assert_eq!(row["props"]["quickReply"][0]["emoji"], ":thumbsup:");
    assert_eq!(row["props"]["quickReply"][0]["userIds"][0], "444");
    assert_eq!(row["reactionCount"], 1);
}

#[test]
fn disabled_is_unsupported_and_goes_directly_remote_without_storage() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Disabled);
    let effects = harness.step(query_tick(1, 20), 1_000);
    assert!(matches!(effects.as_slice(), [EffectKind::Http(_)]));
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::Persist { .. } | Effect::PersistFire { .. }) }));
    let http_corr = corr_for_http(harness.sink.as_slice());

    let effects = harness.step(
        port_reply_with_body(
            http_corr,
            latest_posts_reply(vec![remote_post(&sid(1), "tmp-1", 100, "remote")]),
        ),
        1_100,
    );
    assert_eq!(effects, vec![EffectKind::Emit]);
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::Persist { .. } | Effect::PersistFire { .. }) }));
}

#[test]
fn http_status_failure_emits_explicit_failure_not_empty_success() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Disabled);
    harness.step(query_tick(1, 20), 1_000);
    let http_corr = corr_for_http(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(http_corr, envelope_with_http_status(503, Vec::new())),
        1_100,
    );

    let events = emitted_values(harness.sink.as_slice());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:post:increment-failed");
    assert_eq!(events[0]["data"]["channelId"], cid(1));
    assert!(events[0]["data"]["error"]
        .as_str()
        .unwrap()
        .contains("HTTP status 503"));
    assert!(events
        .iter()
        .all(|event| event["event"] != "im:messages:query_result"));
}

#[test]
fn local_row_count_equal_to_limit_without_window_proof_is_still_partial() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    harness.step(query_tick(1, 2), 1_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    let rows = Bytes::from(
        serde_json::to_vec(&json!([
            local_row(&sid(2), "tmp-2", 200, "newer"),
            local_row(&sid(1), "tmp-1", 100, "older"),
        ]))
        .unwrap(),
    );
    let effects = harness.step(port_reply_with_body(scan_corr, rows), 1_100);
    assert!(matches!(effects.as_slice(), [EffectKind::Http(_)]));
    assert!(emitted_values(harness.sink.as_slice()).is_empty());
}

#[test]
fn exhausted_remote_empty_drops_stale_server_rows_and_keeps_optimistic_row() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Session);
    harness.step(query_tick(1, 20), 1_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    let local = Bytes::from(
        serde_json::to_vec(&json!([
            optimistic_row("tmp-optimistic", 300, "pending"),
            local_row(&sid(1), "tmp-stale", 200, "stale"),
        ]))
        .unwrap(),
    );
    harness.step(port_reply_with_body(scan_corr, local), 1_100);
    let http_corr = corr_for_http(harness.sink.as_slice());
    let effects = harness.step(
        port_reply_with_body(http_corr, latest_posts_reply(Vec::new())),
        1_200,
    );
    assert!(matches!(effects.as_slice(), [EffectKind::Persist(_)]));
    assert!(harness.sink.as_slice().iter().any(|effect| {
        matches!(effect, Effect::Persist { ops, .. }
            if ops.iter().any(|op| matches!(op, StorageOp::BatchDelete(spec)
                if spec.table == "message" && spec.scope_col == "channel_id")))
    }));

    let cache_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(port_reply_ok(cache_corr), 1_300);
    let events = emitted_values(harness.sink.as_slice());
    let messages = events[0]["data"]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["temporaryId"], "tmp-optimistic");
}

#[test]
fn disconnect_drops_late_scan_http_and_cache_replies() {
    // late Scan
    let mut scan = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    scan.step(query_tick(1, 20), 1_000);
    let scan_corr = corr_for_persist(scan.sink.as_slice());
    scan.step(Tick::Disconnected(TransportId::from_raw(1)), 1_050);
    assert!(scan
        .step(
            port_reply_with_body(scan_corr, Bytes::from_static(b"[]")),
            1_100,
        )
        .is_empty());

    // late HTTP
    let mut http = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    http.step(query_tick(1, 20), 2_000);
    let local_corr = corr_for_persist(http.sink.as_slice());
    http.step(
        port_reply_with_body(local_corr, Bytes::from_static(b"[]")),
        2_100,
    );
    let http_corr = corr_for_http(http.sink.as_slice());
    http.step(Tick::Disconnected(TransportId::from_raw(1)), 2_150);
    assert!(http
        .step(
            port_reply_with_body(
                http_corr,
                latest_posts_reply(vec![remote_post(&sid(1), "tmp-1", 100, "late")]),
            ),
            2_200,
        )
        .is_empty());

    // late cache Persist
    let mut cache = Harness::new_with_store_mode(&[1], LocalStoreMode::Session);
    cache.step(query_tick(1, 20), 3_000);
    let local_corr = corr_for_persist(cache.sink.as_slice());
    cache.step(
        port_reply_with_body(local_corr, Bytes::from_static(b"[]")),
        3_100,
    );
    let http_corr = corr_for_http(cache.sink.as_slice());
    cache.step(
        port_reply_with_body(
            http_corr,
            latest_posts_reply(vec![remote_post(&sid(1), "tmp-1", 100, "late")]),
        ),
        3_200,
    );
    let cache_corr = corr_for_persist(cache.sink.as_slice());
    cache.step(Tick::Disconnected(TransportId::from_raw(1)), 3_250);
    assert!(cache.step(port_reply_ok(cache_corr), 3_300).is_empty());
}

#[test]
fn cache_failure_emits_remote_result_but_does_not_create_coverage() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Session);
    harness.step(query_tick(1, 2), 1_000);
    let local_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(local_corr, Bytes::from_static(b"[]")),
        1_100,
    );
    let http_corr = corr_for_http(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(
            http_corr,
            latest_posts_reply(vec![remote_post(&sid(1), "tmp-1", 100, "remote")]),
        ),
        1_200,
    );
    let cache_corr = corr_for_persist(harness.sink.as_slice());
    assert_eq!(
        harness.step(port_reply_err(cache_corr), 1_300),
        vec![EffectKind::Emit]
    );
    let events = emitted_values(harness.sink.as_slice());
    assert_eq!(events[0]["event"], "im:messages:query_result");
    assert_eq!(events[0]["data"]["messages"][0]["msgId"], sid(1));

    harness.step(query_tick(1, 2), 2_000);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    let rows = Bytes::from(
        serde_json::to_vec(&json!([local_row(&sid(1), "tmp-1", 100, "remote")])).unwrap(),
    );
    assert!(matches!(
        harness
            .step(port_reply_with_body(scan_corr, rows), 2_100)
            .as_slice(),
        [EffectKind::Http(_)]
    ));
}

#[test]
fn message_ws_fact_invalidates_coverage_before_persist_fire_completes() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Durable);
    harness.step(query_tick(1, 2), 1_000);
    let local_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(local_corr, Bytes::from_static(b"[]")),
        1_100,
    );
    let http_corr = corr_for_http(harness.sink.as_slice());
    harness.step(
        port_reply_with_body(
            http_corr,
            latest_posts_reply(vec![
                remote_post(&sid(1), "tmp-1", 100, "older"),
                remote_post(&sid(2), "tmp-2", 200, "newer"),
            ]),
        ),
        1_200,
    );
    let cache_corr = corr_for_persist(harness.sink.as_slice());
    harness.step(port_reply_ok(cache_corr), 1_300);

    let inbound = json!({
        "action": "post",
        "data": {
            "id": sid(3),
            "temporaryId": "tmp-3",
            "channelId": cid(1),
            "userId": "remote-user",
            "type": "TEXT",
            "message": "newest",
            "viewers": ["all"],
            "event_seq": 1,
            "createAt": 300,
        }
    });
    harness.step(
        Tick::Inbound(helix_core::tick::InboundBytes(
            serde_json::to_vec(&inbound).unwrap().into(),
        )),
        1_400,
    );

    harness.step(query_tick(1, 2), 1_500);
    let scan_corr = corr_for_persist(harness.sink.as_slice());
    let stale_rows = Bytes::from(
        serde_json::to_vec(&json!([
            local_row(&sid(2), "tmp-2", 200, "newer"),
            local_row(&sid(1), "tmp-1", 100, "older"),
        ]))
        .unwrap(),
    );
    assert!(matches!(
        harness
            .step(port_reply_with_body(scan_corr, stale_rows), 1_600)
            .as_slice(),
        [EffectKind::Http(_)]
    ));
}

#[test]
fn cross_channel_remote_post_is_rejected_without_cache_or_success_projection() {
    let mut harness = Harness::new_with_store_mode(&[1], LocalStoreMode::Disabled);
    harness.step(query_tick(1, 20), 1_000);
    let http_corr = corr_for_http(harness.sink.as_slice());
    let mut wrong_channel = remote_post(&sid(2), "tmp-2", 200, "wrong");
    wrong_channel["channelId"] = Value::String(cid(2));
    let effects = harness.step(
        port_reply_with_body(http_corr, latest_posts_reply(vec![wrong_channel])),
        1_100,
    );
    assert_eq!(effects, vec![EffectKind::Emit]);
    let events = emitted_values(harness.sink.as_slice());
    assert_eq!(events[0]["event"], "im:post:increment-failed");
    assert!(events
        .iter()
        .all(|event| event["event"] != "im:messages:query_result"));
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. } | Effect::PersistFire { .. })));
}
