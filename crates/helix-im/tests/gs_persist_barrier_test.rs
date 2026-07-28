//! GS01-04 最终投影持久化屏障。
//!
//! HTTP/WS 只提供输入事实；最终业务投影必须由 Helix 在同批本地事务成功回执后发布。

mod common;
use common::*;

use helix_core::effect::{StorageOp, TransportId};
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use helix_im::state::Seq;
use helix_im::sync_session::IncrementChannel;
use serde_json::{json, Value};

fn event_name(effect: &Effect) -> Option<String> {
    let Effect::Emit { event } = effect else {
        return None;
    };
    serde_json::from_slice::<Value>(event.0.as_ref()).ok()?["event"]
        .as_str()
        .map(str::to_string)
}

fn event_value(effect: &Effect, expected: &str) -> Option<Value> {
    let Effect::Emit { event } = effect else {
        return None;
    };
    let value: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
    (value["event"] == expected).then_some(value)
}

fn emitted(sink: &helix_core::EffectSink, expected: &str) -> bool {
    sink.as_slice()
        .iter()
        .any(|effect| event_name(effect).as_deref() == Some(expected))
}

fn pong_gap(channel_num: u64) -> Tick {
    let payload = json!({
        "status": "OK",
        "seq": 42,
        "data": {
            "gaps": [{"channelId": cid(channel_num), "fromSeq": 1, "maxSeq": 9}],
            "hashMismatch": false
        }
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

fn increment(channel_num: u64, last_event_seq: u64, need_sync: bool) -> IncrementChannel {
    let channel_id = channel_id(channel_num);
    IncrementChannel {
        channel_id,
        last_event_seq: Seq(last_event_seq),
        need_sync,
        raw: bytes::Bytes::from(
            serde_json::to_vec(&json!({
                "id": channel_id.as_str(),
                "displayName": "persist barrier",
                "lastEventSeq": last_event_seq,
                "needSync": need_sync,
                "members": [{"id": "member-a"}],
                "mentionList": [],
                "urgentPostList": []
            }))
            .unwrap(),
        ),
    }
}

fn table_name(op: &StorageOp) -> Option<&'static str> {
    match op {
        StorageOp::BatchUpsert(spec) => Some(spec.table),
        StorageOp::BatchUpdate(spec) => Some(spec.table),
        StorageOp::BatchDelete(spec) => Some(spec.table),
        StorageOp::MonotonicUpsert(spec) => Some(spec.table),
        StorageOp::GuardedBump(spec) => Some(spec.table),
        StorageOp::Get(spec) => Some(spec.table),
        StorageOp::Scan(spec) => Some(spec.table),
    }
}

fn increment_batch_persist(h: &Harness) -> (u64, Vec<&'static str>) {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| table_name(op) == Some("channel")) =>
            {
                Some((
                    corr.raw(),
                    ops.iter().filter_map(table_name).collect::<Vec<_>>(),
                ))
            }
            _ => None,
        })
        .expect("increment end 必须形成带回执的同批事务")
}

fn prepare_increment_batch() -> Harness {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 10);
    h.step(hello_inbound("CONN-GS1"), 20);
    h.sink.clear();
    h.module
        .ingest_increment(&increment(41, 0, false), &mut h.sink);
    assert!(!emitted(&h.sink, "im:channel:increment"));
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    h
}

#[test]
fn gs1_http_and_increment_end_do_not_publish_before_same_batch_commit() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 10);
    h.step(hello_inbound("CONN-GS1"), 20);

    assert!(emitted(&h.sink, "im:connection:established"));
    assert!(!emitted(&h.sink, "im:channels:loaded"));
    let http_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channels/load/increment") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("hello 必须触发 increment HTTP");
    h.step(port_reply_ok(http_corr), 30);
    assert!(!emitted(&h.sink, "im:channels:loaded"));

    h.sink.clear();
    h.module
        .ingest_increment(&increment(41, 0, false), &mut h.sink);
    assert!(!emitted(&h.sink, "im:channel:increment"));
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Persist { .. })));

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let (_, tables) = increment_batch_persist(&h);
    assert!(!emitted(&h.sink, "im:channel:increment"));
    assert!(!emitted(&h.sink, "im:channels:loaded"));
    let channel = tables.iter().position(|table| *table == "channel").unwrap();
    let member = tables
        .iter()
        .position(|table| *table == "channel_member")
        .unwrap();
    let cursor = tables
        .iter()
        .position(|table| *table == "channel_event_cursor")
        .unwrap();
    assert!(
        channel < member && member < cursor,
        "事务顺序必须 channel -> member -> cursor，got {tables:?}"
    );
}

#[test]
fn gs1_persist_error_never_publishes_final_projection() {
    let mut h = prepare_increment_batch();
    let (persist_corr, _) = increment_batch_persist(&h);
    h.step(port_reply_err(persist_corr), 100);
    assert!(!emitted(&h.sink, "im:channel:increment"));
    assert!(!emitted(&h.sink, "im:channels:loaded"));
}

#[test]
fn gs1_persist_ok_publishes_increment_then_loaded() {
    let mut h = prepare_increment_batch();
    let (persist_corr, _) = increment_batch_persist(&h);
    h.step(port_reply_ok(persist_corr), 100);
    let events: Vec<String> = h.sink.as_slice().iter().filter_map(event_name).collect();
    assert_eq!(
        events,
        vec!["im:channel:increment", "im:channels:loaded"],
        "只有事务成功回执才能按 increment -> loaded 发布"
    );
}

#[test]
/// 证明已 attach 的 dialog 只在 GS1 PersistOk 后从 durable channel 表重新扫描。
fn gs1_persist_ok_refreshes_attached_dialog_from_storage() {
    let mut h = Harness::new_with_identity(&[], "gs1-actor", "gs1-team", "", "", "");
    h.step(command_json("im_query_dialog_list", json!({})), 1);
    let attach_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel")) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("dialog attach must scan channel");
    h.step(
        port_reply_with_body(attach_corr, bytes::Bytes::from_static(b"[]")),
        2,
    );
    h.step(hello_inbound("CONN-GS1-ATTACHED"), 3);
    h.module
        .ingest_increment(&increment(41, 0, false), &mut h.sink);
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let (persist_corr, _) = increment_batch_persist(&h);
    assert!(
        h.sink.as_slice().iter().all(|effect| !matches!(
            effect,
            Effect::Persist { ops, .. }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"),
                )
        )),
        "global end must not scan before batch PersistOk"
    );

    h.step(port_reply_ok(persist_corr), 4);
    assert!(
        h.sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Persist { ops, .. }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::Scan(scan) if scan.table == "channel"),
                )
        )),
        "batch PersistOk must schedule one durable dialog scan"
    );
}

#[test]
fn gs1_persists_all_channels_but_bounds_initial_projection_window() {
    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 10);
    h.step(hello_inbound("CONN-GS1-BOUND"), 20);
    h.sink.clear();
    for channel_num in 1..=501 {
        h.module
            .ingest_increment(&increment(channel_num, 0, false), &mut h.sink);
    }
    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);
    let (persist_corr, tables) = increment_batch_persist(&h);
    assert!(
        tables.iter().filter(|table| **table == "channel").count() >= 501,
        "首屏窗口不得裁掉同批 SQLite channel 写入"
    );

    h.step(port_reply_ok(persist_corr), 100);
    let events: Vec<String> = h.sink.as_slice().iter().filter_map(event_name).collect();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.as_str() == "im:channel:increment")
            .count(),
        500,
        "首屏逐频道投影必须受 dialog window 上限约束"
    );
    assert_eq!(
        events.last().map(String::as_str),
        Some("im:channels:loaded")
    );
}

fn prepare_sync_persist() -> (Harness, u64, Vec<&'static str>) {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let sync_corr = h
        .step(inbound_event(1, 99, 1), 1_000)
        .into_iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(corr),
            _ => None,
        })
        .expect("gap 必须触发 sync/notify");
    let response = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "message-gs")],
        &[("message-gs", "offline message", "")],
        1,
    );
    h.step(port_reply_with_body(sync_corr, enveloped(response)), 1_100);
    assert!(
        h.sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Emit { .. })),
        "HTTP SUCCESS/解析成功后、持久化回执前不得发布任何 sync 最终投影"
    );
    let (corr, tables) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => Some((
                corr.raw(),
                ops.iter().filter_map(table_name).collect::<Vec<_>>(),
            )),
            _ => None,
        })
        .expect("sync events 必须形成带回执事务");
    (h, corr, tables)
}

#[test]
fn gs2_gs3_sync_transaction_is_atomic_and_rolls_back_facts_ledger_coverage_and_cursor() {
    let (mut h, persist_corr, tables) = prepare_sync_persist();
    let message = tables.iter().position(|table| *table == "message").unwrap();
    let cursor = tables
        .iter()
        .position(|table| *table == "channel_event_cursor")
        .unwrap();
    let ledger = tables
        .iter()
        .position(|table| *table == "channel_event_ledger")
        .unwrap();
    let coverage = tables
        .iter()
        .position(|table| *table == "channel_sync_coverage")
        .unwrap();
    assert!(
        message < ledger && ledger < coverage && coverage < cursor,
        "sync 原子事务必须 facts -> ledger -> coverage -> cursor，got {tables:?}"
    );

    h.step(port_reply_err(persist_corr), 1_200);
    assert!(h
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Emit { .. })));
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(0));
}

#[test]
fn gs2_gs3_sync_commit_publishes_mutation_dialog_and_sync_state() {
    let (mut h, persist_corr, _) = prepare_sync_persist();
    let channel_row = bytes::Bytes::from(
        serde_json::to_vec(&json!([{
            "unread_count": 1,
            "mention_count": 0,
            "urgent_count": 0,
            "mention_list": [],
            "urgent_post_list": [],
            "unread_post_id": "message-gs",
            "last_root_post_at": 100
        }]))
        .unwrap(),
    );
    h.step(port_reply_with_body(persist_corr, channel_row), 1_200);
    assert!(emitted(&h.sink, "im:post:received"));
    assert!(!emitted(&h.sink, "im:channel:update-by-post"));
    let sync_state = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| event_value(effect, "im:sync:state"))
        .expect("事务成功后必须发布 sync state");
    assert_eq!(sync_state["data"]["state"], "committed");
    assert_eq!(sync_state["data"]["recovery"]["channelId"], cid(1));
    assert_eq!(sync_state["data"]["recovery"]["committedSeq"], 1);
    assert!(
        sync_state["data"].get("trigger").is_none(),
        "普通 gap sync 的既有 im:sync:state schema 不得新增 trigger"
    );
    assert_eq!(h.module.cursor_for(channel_id(1)), Some(1));
}

#[test]
fn gs3_pong_gap_trigger_survives_http_and_persist_barrier() {
    let mut h = Harness::new_with_identity(&[1], "viewer-444", "tenant-1", "", "", "");
    let sync_corr = h
        .step(pong_gap(1), 1_000)
        .into_iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(corr),
            _ => None,
        })
        .expect("pong gap 必须触发 sync/notify");
    let response = sync_reply_events_with_messages(
        1,
        &[ev(1, 1, "message-pong-gap")],
        &[("message-pong-gap", "pong offline message", "")],
        1,
    );
    h.step(port_reply_with_body(sync_corr, enveloped(response)), 1_100);
    assert!(!emitted(&h.sink, "im:sync:state"));
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("pong gap sync 必须形成持久化屏障");
    let channel_row = bytes::Bytes::from(
        serde_json::to_vec(&json!([{
            "unread_count": 1,
            "mention_count": 0,
            "urgent_count": 0,
            "mention_list": [],
            "urgent_post_list": [],
            "unread_post_id": "message-pong-gap",
            "last_root_post_at": 100
        }]))
        .unwrap(),
    );
    h.step(port_reply_with_body(persist_corr, channel_row), 1_200);
    let sync_state = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| event_value(effect, "im:sync:state"))
        .expect("pong gap 事务成功后必须发布 sync state");
    assert_eq!(sync_state["data"]["trigger"], "pong_gap");
    assert_eq!(sync_state["data"]["recovery"]["committedSeq"], 1);
}

fn hydration_reply() -> bytes::Bytes {
    bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "id": cid(45),
                "displayName": "unknown channel",
                "lastEventSeq": 0,
                "needSync": false,
                "members": [{"id": "member-a"}]
            }
        }))
        .unwrap(),
    )
}

fn prepare_hydration_persist() -> (Harness, u64) {
    let mut h = Harness::new(&[]);
    let http_corr = h
        .step(
            command_json(
                "im_channel_load_increment_by_channel_id",
                json!({"channel_id": cid(45), "req_id": "req-gs4"}),
            ),
            2_000,
        )
        .into_iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(corr),
            _ => None,
        })
        .expect("单频道 hydration 必须发 HTTP");
    h.step(
        port_reply_with_body(http_corr, enveloped(hydration_reply())),
        2_100,
    );
    assert!(emitted(&h.sink, "im:read:result"));
    assert!(!emitted(&h.sink, "im:channel:increment"));
    assert!(!emitted(&h.sink, "im:channels:projection"));
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| table_name(op) == Some("channel")) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("hydration 必须等待 channel/member Persist");
    (h, persist_corr)
}

#[test]
fn gs4_hydration_projection_is_released_only_by_persist_ok() {
    let (mut failed, failed_corr) = prepare_hydration_persist();
    failed.step(port_reply_err(failed_corr), 2_200);
    assert!(!emitted(&failed.sink, "im:channel:increment"));
    assert!(!emitted(&failed.sink, "im:channels:projection"));

    let (mut committed, committed_corr) = prepare_hydration_persist();
    committed.step(port_reply_ok(committed_corr), 2_200);
    assert!(emitted(&committed.sink, "im:channel:increment"));
    assert!(!emitted(&committed.sink, "im:channels:projection"));
}
