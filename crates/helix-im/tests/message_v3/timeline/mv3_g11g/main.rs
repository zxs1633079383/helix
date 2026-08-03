//! MV3-G11g：exact-by-id 只经 HTTP → Persist → 单键 Scan read-back 结算。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::Correlation;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_err, port_reply_with_body, Harness};

const CHANNEL: u64 = 0x11f;

/// 从当前批次取 exact HTTP correlation，并确认没有额外 HTTP。
fn only_http(harness: &Harness) -> Correlation {
    let mut values = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/get") => Some(*corr),
            _ => None,
        });
    let corr = values.next().expect("expected exact posts HTTP");
    assert!(values.next().is_none(), "exact request must issue one HTTP");
    corr
}

/// 从当前批次取唯一 exact cache Persist correlation。
fn only_cache_persist(harness: &Harness) -> Correlation {
    let mut values = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"),
                ) =>
            {
                Some(*corr)
            }
            _ => None,
        });
    let corr = values.next().expect("expected exact cache Persist");
    assert!(
        values.next().is_none(),
        "exact request must issue one cache Persist"
    );
    corr
}

/// 从当前批次取唯一 exact durable Scan correlation。
fn only_scan(harness: &Harness) -> Correlation {
    let mut values = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Scan(spec) if spec.table == "message")) =>
            {
                Some(*corr)
            }
            _ => None,
        });
    let corr = values.next().expect("expected exact durable Scan");
    assert!(
        values.next().is_none(),
        "exact request must issue one Scan per stage"
    );
    corr
}

/// 解出当前批次所有 Emit，确保只产生 im:read:result 终态。
fn emits(harness: &Harness) -> Vec<Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// 构造可见、不可见和重复/缺失 identity，验证按 id/temporaryId 命中且 viewer 过滤。
fn exact_posts() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": [
                {
                    "id": "srvfix00000000000000000001",
                    "temporaryId": "tmp-visible",
                    "channelId": cid(CHANNEL),
                    "userId": "sender-1",
                    "type": "TEXT",
                    "message": "http-visible",
                    "simpleMessage": "http-visible",
                    "viewers": ["viewer-1"]
                },
                {
                    "id": "srvfix00000000000000000002",
                    "temporaryId": "tmp-hidden",
                    "channelId": cid(CHANNEL),
                    "userId": "sender-2",
                    "type": "TEXT",
                    "message": "http-hidden",
                    "viewers": ["other-viewer"]
                }
            ]
        }))
        .unwrap(),
    )
}

/// 构造与 HTTP 内容不同的 durable row，证明终态来自本地 read-back。
fn durable_visible_row() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!([{
            "id": "srvfix00000000000000000001",
            "temporary_id": "tmp-visible",
            "channel_id": cid(CHANNEL),
            "user_id": "sender-1",
            "type": "TEXT",
            "message": "durable-visible",
            "simple_message": "durable-visible",
            "props": "{\"tenant\":\"t1\"}",
            "user_snapshot": "{\"userId\":\"sender-1\"}",
            "viewers": "[\"viewer-1\"]",
            "create_at": 1700000000000_i64,
            "event_seq": 99,
            "read_bits": "raw-bitmap-must-not-leak",
            "send_status": "sent"
        }]))
        .unwrap(),
    )
}

#[test]
fn g11g_exact_posts_waits_for_durable_readback_and_filters_viewer() {
    let mut harness = Harness::new_with_identity(&[], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts",
            json!({
                "req_id": "req-g11g-1",
                "post_ids": ["srvfix00000000000000000001", "tmp-hidden", "missing", "srvfix00000000000000000001"],
                "locate": true
            }),
        ),
        1,
    );
    let http = only_http(&harness);
    let body = harness.last_http_body().expect("exact HTTP body");
    assert_eq!(
        body["postIds"],
        json!(["srvfix00000000000000000001", "tmp-hidden", "missing"])
    );
    assert!(
        body.get("locate").is_none(),
        "locate must not affect wire request"
    );
    assert!(emits(&harness).is_empty(), "HTTP reply is not terminal");

    harness.step(
        port_reply_with_body(http.raw(), enveloped(exact_posts())),
        2,
    );
    let persist = only_cache_persist(&harness);
    assert!(
        emits(&harness).is_empty(),
        "Persist barrier must precede terminal"
    );
    harness.step(port_reply_with_body(persist.raw(), Bytes::new()), 3);
    let scan = only_scan(&harness);
    let scan_spec = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } if *corr == scan => ops.iter().find_map(|op| match op {
                StorageOp::Scan(spec) => Some(spec),
                _ => None,
            }),
            _ => None,
        })
        .expect("exact scan spec");
    assert_eq!(
        scan_spec.filter.as_ref().map(|(field, _)| *field),
        Some("temporary_id")
    );
    harness.step(port_reply_with_body(scan.raw(), durable_visible_row()), 4);
    let events = emits(&harness);
    assert_eq!(events.len(), 1, "one reqId has one terminal");
    let event = &events[0];
    assert_eq!(event["event"], "im:read:result");
    assert_eq!(event["data"]["req_id"], "req-g11g-1");
    assert_eq!(event["data"]["body"]["reqId"], "req-g11g-1");
    assert_eq!(
        event["data"]["body"]["posts"].as_array().map(Vec::len),
        Some(1)
    );
    let post = &event["data"]["body"]["posts"][0];
    assert_eq!(post["message"], "durable-visible");
    assert!(post.get("readBits").is_none());
    assert!(post.get("eventSeq").is_none());
    assert!(post.get("olderCursor").is_none());
}

#[test]
fn g11g_empty_ids_emit_one_empty_terminal_after_empty_persist() {
    let mut harness = Harness::new_with_identity(&[], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts",
            json!({"req_id": "req-g11g-empty", "post_ids": [], "locate": true}),
        ),
        1,
    );
    let http = only_http(&harness);
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from_static(br#"{"status":"SUCCESS","data":[]}"#)),
        ),
        2,
    );
    let persist = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } if ops.is_empty() => Some(*corr),
            _ => None,
        })
        .expect("empty exact request must retain correlation barrier");
    assert!(emits(&harness).is_empty());
    harness.step(port_reply_with_body(persist.raw(), Bytes::new()), 3);
    let events = emits(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["body"]["posts"], json!([]));
}

#[test]
fn g11g_http_failure_has_one_error_terminal() {
    let mut harness = Harness::new_with_identity(&[], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts",
            json!({"req_id": "req-g11g-fail", "post_ids": ["missing"]}),
        ),
        1,
    );
    let http = only_http(&harness);
    harness.step(port_reply_err(http.raw()), 2);
    let events = emits(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], "req-g11g-fail");
    assert!(events[0]["data"].get("body").is_none());
    assert!(events[0]["data"].get("error").is_some());
}
