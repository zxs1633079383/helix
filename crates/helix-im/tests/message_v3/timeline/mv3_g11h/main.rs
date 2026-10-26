//! MV3-G11h：只知 postId 的 initial window 经过 HTTP → Persist → durable read-back。

use bytes::Bytes;
use helix_core::effect::{Effect, ScanOrder, StorageOp};
use helix_core::Correlation;
use helix_core::Module;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, Harness};

const TARGET: &str = "srvfix00000000000000000011";
const TARGET_TMP: &str = "tmp-initial-target";
const NEXT: &str = "srvfix00000000000000000012";
const NEXT_TMP: &str = "tmp-initial-next";
const CHANNEL: u64 = 0x11f;
const REQUEST_ID: &str = "req-g11h-1";

/// 从当前批次取唯一 initial-window HTTP correlation。
fn only_http(harness: &Harness) -> Correlation {
    let mut values = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getPostsAfterIndex") => {
                Some(*corr)
            }
            _ => None,
        });
    let corr = values.next().expect("expected initial-window HTTP");
    assert!(
        values.next().is_none(),
        "initial request must issue one HTTP"
    );
    corr
}

/// 从当前批次取唯一 message cache Persist correlation。
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
    let corr = values
        .next()
        .expect("expected initial-window cache Persist");
    assert!(
        values.next().is_none(),
        "initial request must issue one cache Persist"
    );
    corr
}

/// 从当前批次取唯一 durable identity Scan correlation。
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
    let corr = values.next().expect("expected initial-window durable Scan");
    assert!(
        values.next().is_none(),
        "initial read-back must issue one Scan per stage"
    );
    corr
}

/// 解出当前批次的所有领域 Emit，供 single-terminal 断言使用。
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

/// 构造 Go initial-window 成功体，目标故意位于首条。
fn initial_posts() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "posts": [
                    {
                        "id": TARGET,
                        "temporaryId": TARGET_TMP,
                        "channelId": cid(CHANNEL),
                        "userId": "sender-1",
                        "type": "TEXT",
                        "message": "target-http",
                        "simpleMessage": "target-http",
                        "viewers": ["viewer-1"]
                    },
                    {
                        "id": NEXT,
                        "temporaryId": NEXT_TMP,
                        "channelId": cid(CHANNEL),
                        "userId": "sender-2",
                        "type": "TEXT",
                        "message": "next-http",
                        "simpleMessage": "next-http",
                        "viewers": ["viewer-1"]
                    }
                ]
            }
        }))
        .unwrap(),
    )
}

/// 构造与 HTTP 内容不同的 durable target row，证明 terminal 只取 read-back。
fn durable_target() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!([{
            "id": TARGET,
            "temporary_id": TARGET_TMP,
            "channel_id": cid(CHANNEL),
            "user_id": "sender-1",
            "type": "TEXT",
            "message": "target-durable",
            "simple_message": "target-durable",
            "props": "{}",
            "user_snapshot": "{}",
            "viewers": "[\"viewer-1\"]",
            "create_at": 1700000000000_i64,
            "send_status": "sent"
        }]))
        .unwrap(),
    )
}

/// 构造与 HTTP 内容不同的 durable next row，维持 initial 页的顺序。
fn durable_next() -> Bytes {
    Bytes::from(
        serde_json::to_vec(&json!([{
            "id": NEXT,
            "temporary_id": NEXT_TMP,
            "channel_id": cid(CHANNEL),
            "user_id": "sender-2",
            "type": "TEXT",
            "message": "next-durable",
            "simple_message": "next-durable",
            "props": "{}",
            "user_snapshot": "{}",
            "viewers": "[\"viewer-1\"]",
            "create_at": 1700000001000_i64,
            "send_status": "sent"
        }]))
        .unwrap(),
    )
}

#[test]
fn g11h_initial_window_waits_for_readback_and_keeps_target_first() {
    let mut harness = Harness::new_with_identity(&[CHANNEL], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts_after_index",
            json!({
                "req_id": REQUEST_ID,
                "post_id": TARGET,
                "page_size": 2
            }),
        ),
        1,
    );
    let http = only_http(&harness);
    let body = harness.last_http_body().expect("initial HTTP body");
    assert_eq!(body["postId"], TARGET);
    assert_eq!(body["direction"], "initial");
    assert_eq!(body["pageSize"], 2);
    assert_eq!(body["reqId"], REQUEST_ID);
    assert!(
        body.get("postIds").is_none(),
        "initial wire must use singular postId"
    );
    assert!(emits(&harness).is_empty(), "HTTP reply is not terminal");

    harness.step(
        port_reply_with_body(http.raw(), enveloped(initial_posts())),
        2,
    );
    let persist = only_cache_persist(&harness);
    assert!(
        emits(&harness).is_empty(),
        "Persist barrier must precede terminal"
    );
    harness.step(port_reply_with_body(persist.raw(), Bytes::new()), 3);

    let first_scan = only_scan(&harness);
    let first_spec = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } if *corr == first_scan => ops.iter().find_map(|op| {
                let StorageOp::Scan(spec) = op else {
                    return None;
                };
                Some(spec)
            }),
            _ => None,
        })
        .expect("target durable Scan spec");
    assert_eq!(first_spec.limit, Some(1));
    assert_eq!(
        first_spec.filter.as_ref().map(|(field, _)| *field),
        Some("temporary_id")
    );
    assert_eq!(first_spec.order_by, &[] as &[ScanOrder]);
    harness.step(port_reply_with_body(first_scan.raw(), durable_target()), 4);

    let second_scan = only_scan(&harness);
    harness.step(port_reply_with_body(second_scan.raw(), durable_next()), 5);
    let events = emits(&harness);
    assert_eq!(events.len(), 1, "one reqId has one terminal");
    let event = &events[0];
    assert_eq!(event["event"], "im:read:result");
    assert_eq!(event["data"]["req_id"], REQUEST_ID);
    assert_eq!(event["data"]["body"]["reqId"], REQUEST_ID);
    let posts = event["data"]["body"]["posts"]
        .as_array()
        .expect("posts array");
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0]["id"], TARGET);
    assert_eq!(posts[0]["message"], "target-durable");
    assert_eq!(posts[1]["id"], NEXT);
    assert_eq!(posts[1]["message"], "next-durable");
    assert!(
        event["data"]["body"].get("generation").is_none(),
        "G11h terminal must not expose Timeline generation"
    );
}

#[test]
fn g11h_target_not_first_fails_closed_without_persist_or_pseudo_success() {
    let mut harness = Harness::new_with_identity(&[CHANNEL], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts_after_index",
            json!({
                "req_id": "req-g11h-order",
                "post_id": TARGET,
                "page_size": 20,
                "direction": "initial"
            }),
        ),
        1,
    );
    let http = only_http(&harness);
    let response = json!({
        "status": "SUCCESS",
        "data": {"posts": [{"id": NEXT, "temporaryId": NEXT_TMP, "channelId": cid(CHANNEL)}, {"id": TARGET, "temporaryId": TARGET_TMP, "channelId": cid(CHANNEL)}]}
    });
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&response).unwrap())),
        ),
        2,
    );
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. })));
    let events = emits(&harness);
    assert_eq!(events.len(), 1);
    assert!(events[0]["data"].get("error").is_some());
    assert!(events[0]["data"].get("body").is_none());
}

#[test]
fn g11h_missing_target_is_typed_empty_terminal_after_empty_persist() {
    let mut harness = Harness::new_with_identity(&[CHANNEL], "viewer-1", "tenant-1", "", "", "");
    harness.step(
        command_json(
            "im_get_posts_after_index",
            json!({
                "req_id": "req-g11h-empty",
                "post_id": "missing",
                "limit": 20
            }),
        ),
        1,
    );
    let http = only_http(&harness);
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from_static(
                br#"{"status":"SUCCESS","data":{"posts":[]}}"#,
            )),
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
        .expect("empty initial request must retain correlation barrier");
    harness.step(port_reply_with_body(persist.raw(), Bytes::new()), 3);
    let events = emits(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["body"]["posts"], json!([]));
}

#[test]
fn g11h_page_size_above_contract_is_rejected_before_http() {
    let mut harness = Harness::new_with_identity(&[CHANNEL], "viewer-1", "tenant-1", "", "", "");
    let command = command_json(
        "im_get_posts_after_index",
        json!({
            "req_id": "req-g11h-invalid",
            "post_id": TARGET,
            "page_size": 61
        }),
    );
    harness
        .module
        .handle(&command, 1, &mut harness.sink)
        .expect_err("pageSize=61 must reject before I/O");
    assert!(harness.sink.as_slice().is_empty());
}
