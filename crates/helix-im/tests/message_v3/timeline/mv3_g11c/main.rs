//! MV3-G11c：newer 只在 durable cache Persist 与 bounded Scan read-back 后结算。

use bytes::Bytes;
use helix_core::effect::{Effect, ScanOrder, StorageOp};
use helix_core::{Correlation, Module, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, sid, Harness};

const CHANNEL: u64 = 0x11c;
const WINDOW: &str = "mv3-g11c-window";
const REQUEST_ID: &str = "req-mv3-g11c-001";

/// 从当前批次提取唯一 newer HTTP correlation，并校验 endpoint。
fn only_http(harness: &Harness) -> Correlation {
    let mut correlations = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getPostsAfterIndex") => {
                Some(*corr)
            }
            _ => None,
        });
    let corr = correlations.next().expect("expected one newer HTTP effect");
    assert!(
        correlations.next().is_none(),
        "newer command must issue one HTTP effect"
    );
    corr
}

/// 从当前批次提取唯一 message cache Persist correlation。
fn only_cache_write(harness: &Harness) -> Correlation {
    let mut correlations = harness
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
    let corr = correlations
        .next()
        .expect("expected one newer cache Persist");
    assert!(
        correlations.next().is_none(),
        "expected one cache Persist only"
    );
    corr
}

/// 从当前批次提取唯一 message Scan correlation。
fn only_message_scan(harness: &Harness) -> Correlation {
    let mut correlations = harness
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
    let corr = correlations.next().expect("expected one message Scan");
    assert!(
        correlations.next().is_none(),
        "expected one message Scan only"
    );
    corr
}

/// 解出当前批次全部 Emit，验证关联查询不产生业务广播。
fn emitted_events(harness: &Harness) -> Vec<Value> {
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

/// 读取当前 HTTP effect body，验证 Rust canonical payload 未透传 Angular 字段。
fn http_body(harness: &Harness, corr: Correlation) -> Value {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http {
                corr: effect_corr,
                req,
            } if *effect_corr == corr => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("newer HTTP body")
}

/// 构造 latest 附着窗口，确保 newer anchor 来自 Helix typed state。
fn attach_latest_window(harness: &mut Harness) {
    harness.step(
        command_json(
            "im_query_messages_by_channel",
            json!({
                "channel_id": cid(CHANNEL),
                "pageSize": 20,
                "windowToken": WINDOW,
            }),
        ),
        1,
    );
    let scan = only_message_scan(harness);
    harness.step(
        port_reply_with_body(scan.raw(), Bytes::from_static(b"[]")),
        2,
    );
    let http = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => Some(*corr),
            _ => None,
        })
        .expect("latest HTTP effect");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(latest_posts())),
        3,
    );
    let cache = only_cache_write(harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 4);
    let readback = only_message_scan(harness);
    harness.step(
        port_reply_with_body(readback.raw(), latest_durable_rows()),
        5,
    );
    assert_eq!(
        emitted_events(harness).len(),
        1,
        "latest must attach one window"
    );
}

/// 构造 canonical newer command，仅携带 Helix 允许的 snake_case 字段。
fn newer_command(page_size: u64) -> helix_core::Tick {
    command_json(
        "im_load_newer_context",
        json!({
            "channel_id": cid(CHANNEL),
            "anchor_post_id": sid(20),
            "page_size": page_size,
            "req_id": REQUEST_ID,
        }),
    )
}

/// 构造 20 条 anchor 之后的 Go newer wire rows。
fn newer_posts() -> Bytes {
    let posts = (21..=40)
        .map(|index| {
            json!({
                "id": sid(index),
                "temporaryId": format!("mv3-g11c-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11c-author",
                "type": "TEXT",
                "message": format!("http-newer-{index}"),
                "simpleMessage": format!("http-newer-{index}"),
                "viewers": ["all"],
                "eventSeq": index,
                "createAt": 1_700_001_200_000_i64 + index as i64,
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {"posts": posts, "hasMore": false}
        }))
        .unwrap(),
    )
}

/// 构造持久化 newer rows，文本故意与 HTTP 不同以证明 Result 来自 read-back。
fn newer_durable_rows() -> Bytes {
    let rows = (21..=40)
        .map(|index| {
            json!({
                "id": sid(index),
                "temporary_id": format!("mv3-g11c-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11c-author",
                "type": "TEXT",
                "message": format!("durable-newer-{index}"),
                "simple_message": format!("durable-newer-{index}"),
                "viewers": ["all"],
                "create_at": 1_700_001_200_000_i64 + index as i64,
                "event_seq": index,
                "send_status": "sent",
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// 构造 latest Go rows，末项 `sid(20)` 是 newer 的 attached anchor。
fn latest_posts() -> Bytes {
    let posts = (1..=20)
        .map(|index| {
            json!({
                "id": sid(index),
                "temporaryId": format!("mv3-g11c-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11c-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simpleMessage": format!("latest-{index}"),
                "viewers": ["all"],
                "eventSeq": index,
                "createAt": 1_700_001_200_000_i64 + index as i64,
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts})).unwrap())
}

/// 构造 latest 的 durable rows，保持相同复合游标以完成 attached state。
fn latest_durable_rows() -> Bytes {
    let rows = (1..=20)
        .map(|index| {
            json!({
                "id": sid(index),
                "temporary_id": format!("mv3-g11c-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11c-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simple_message": format!("latest-{index}"),
                "viewers": ["all"],
                "create_at": 1_700_001_200_000_i64 + index as i64,
                "event_seq": index,
                "send_status": "sent",
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// MV3-G11c 成功链严格为 HTTP → Persist → Scan → 唯一 QueryMessagesResult。
#[test]
fn g11c_newer_waits_for_durable_scan_and_returns_sorted_result() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(newer_command(20), 6);
    let http = only_http(&harness);
    let body = http_body(&harness, http);
    assert_eq!(body["postIds"], sid(20));
    assert_eq!(body["direction"], "newer");
    assert_eq!(body["pageSize"], 20);
    assert_eq!(body["cursorVersion"], 1);
    assert!(body.get("limit").is_none());
    assert!(body.get("req_id").is_none());
    assert!(emitted_events(&harness).is_empty(), "HTTP is not terminal");

    harness.step(
        port_reply_with_body(http.raw(), enveloped(newer_posts())),
        7,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "cache Persist is not terminal"
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    assert!(
        emitted_events(&harness).is_empty(),
        "PersistOk is not terminal"
    );
    let readback = only_message_scan(&harness);
    let scan = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } if *corr == readback => ops.iter().find_map(|op| {
                let StorageOp::Scan(spec) = op else {
                    return None;
                };
                Some(spec)
            }),
            _ => None,
        })
        .expect("newer bounded Scan spec");
    assert_eq!(scan.limit, Some(21));
    match &scan.filter {
        Some((column, helix_core::effect::SqlValue::Text(channel))) => {
            assert_eq!(*column, "channel_id");
            assert_eq!(channel.as_str(), cid(CHANNEL));
        }
        _ => panic!("newer Scan must filter the requested channel"),
    }
    assert_eq!(
        scan.order_by,
        &[ScanOrder::asc("create_at"), ScanOrder::asc("temporary_id")]
    );
    harness.step(
        port_reply_with_body(readback.raw(), newer_durable_rows()),
        9,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "one reqId must have one terminal result");
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert_eq!(events[0]["data"]["body"]["reqId"], REQUEST_ID);
    assert_eq!(
        events[0]["data"]["body"]["messages"]
            .as_array()
            .map(Vec::len),
        Some(20)
    );
    assert_eq!(
        events[0]["data"]["body"]["messages"][0]["text"],
        "durable-newer-21"
    );
    assert_eq!(
        events[0]["data"]["body"]["messages"][19]["text"],
        "durable-newer-40"
    );
    assert_eq!(events[0]["data"]["body"]["hasNewer"], false);
    assert!(
        events[0]["data"]["body"].get("generation").is_none(),
        "G11c terminal must not expose Timeline generation"
    );
    let encoded = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "postReceived",
        "postIncrement",
        "postIncrementStarted",
        "postIncrementEmpty",
        "postIncrementFailed",
        "im:timeline:update",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "forbidden newer output: {forbidden}"
        );
    }
}

/// newer 空页也必须经过空 Persist 与 bounded Scan，且成功结果保留旧窗口。
#[test]
fn g11c_empty_newer_page_is_valid_terminal_without_broadcast() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(newer_command(20), 6);
    let http = only_http(&harness);
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from_static(
                br#"{"status":"SUCCESS","data":{"posts":[],"hasMore":false}}"#,
            )),
        ),
        7,
    );
    assert!(emitted_events(&harness).is_empty());
    let readback = only_message_scan(&harness);
    harness.step(
        port_reply_with_body(readback.raw(), Bytes::from_static(b"[]")),
        9,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert_eq!(events[0]["data"]["body"]["messages"], json!([]));
    assert_eq!(events[0]["data"]["body"]["hasNewer"], false);
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("im:timeline:update"));
}

/// newer 页长 60 合法，超过上限或缺失 reqId 时在任何 I/O 前拒绝。
#[test]
fn g11c_page_size_and_req_id_fail_closed_before_io() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(newer_command(60), 6);
    assert_eq!(http_body(&harness, only_http(&harness))["pageSize"], 60);
    harness.sink.clear();
    let invalid = newer_command(61);
    harness
        .module
        .handle(&invalid, 7, &mut harness.sink)
        .expect_err("pageSize=61 must reject");
    assert!(harness.sink.as_slice().is_empty());
    let missing_req = command_json(
        "im_load_newer_context",
        json!({
            "channel_id": cid(CHANNEL),
            "anchor_post_id": sid(20),
            "page_size": 20,
        }),
    );
    harness
        .module
        .handle(&missing_req, 8, &mut harness.sink)
        .expect_err("missing req_id must reject");
    assert!(harness.sink.as_slice().is_empty());
}

/// HTTP、Persist 与 read-back 各阶段失败都只返回同 reqId error，不擦除旧窗口或广播。
#[test]
fn g11c_failures_are_correlated_and_preserve_old_window() {
    let mut http_failure =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut http_failure);
    http_failure.step(newer_command(20), 6);
    let http = only_http(&http_failure);
    http_failure.step(common::port_reply_err(http.raw()), 7);
    let events = emitted_events(&http_failure);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert!(events[0]["data"].get("error").is_some());
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("im:timeline:update"));

    let mut persist_failure =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut persist_failure);
    persist_failure.step(newer_command(20), 6);
    let http = only_http(&persist_failure);
    persist_failure.step(
        port_reply_with_body(http.raw(), enveloped(newer_posts())),
        7,
    );
    let cache = only_cache_write(&persist_failure);
    persist_failure.step(
        Tick::PortReply {
            corr: cache,
            outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Storage(500)),
        },
        8,
    );
    let events = emitted_events(&persist_failure);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert!(events[0]["data"].get("error").is_some());
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("im:timeline:update"));

    let mut readback_failure =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut readback_failure);
    readback_failure.step(newer_command(20), 6);
    let http = only_http(&readback_failure);
    readback_failure.step(
        port_reply_with_body(http.raw(), enveloped(newer_posts())),
        7,
    );
    let cache = only_cache_write(&readback_failure);
    readback_failure.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let readback = only_message_scan(&readback_failure);
    readback_failure.step(
        Tick::PortReply {
            corr: readback,
            outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Storage(500)),
        },
        9,
    );
    let events = emitted_events(&readback_failure);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert!(events[0]["data"].get("error").is_some());
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("im:timeline:update"));
}

/// matching read-back correlation 被消费后，迟到重复回包必须保持 no-op。
#[test]
fn g11c_duplicate_readback_is_noop() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(newer_command(20), 6);
    let http = only_http(&harness);
    harness.step(
        port_reply_with_body(http.raw(), enveloped(newer_posts())),
        7,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let readback = only_message_scan(&harness);
    harness.step(
        port_reply_with_body(readback.raw(), newer_durable_rows()),
        9,
    );
    assert_eq!(emitted_events(&harness).len(), 1);
    harness.step(
        port_reply_with_body(readback.raw(), newer_durable_rows()),
        10,
    );
    assert!(harness.sink.as_slice().is_empty());
}
