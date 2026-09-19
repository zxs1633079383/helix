//! MV3-G11b：older Timeline 只在 durable cache Persist 与 bounded Scan read-back 后结算。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::{Correlation, Module};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL: u64 = 0x11b;
const WINDOW: &str = "mv3-g11b-window";
const REQUEST_ID: &str = "req-mv3-g11b-001";

/// 为 older 回放注入真实 runtime identity 与 channel scope，保持正例可进入渲染闸门。
fn scoped_harness() -> Harness {
    let config = helix_im::module::ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        auth_user_id: "g11b-viewer".to_string(),
        company_id: "g11b-company".to_string(),
        ..Default::default()
    };
    let mut harness = Harness {
        module: helix_im::ImModule::new_with_local_store(
            config,
            helix_im::query::LocalStoreMode::Session,
        ),
        sink: helix_core::EffectSink::new(),
        effect_log: Vec::new(),
    };
    harness.module.register_channel(common::channel_id(CHANNEL), 0);
    harness.seed_channel_scope(&common::cid(CHANNEL), "g11b-company", "g11b-viewer");
    harness.sink.clear();
    harness.effect_log.clear();
    harness
}

/// 从当前批次提取唯一 HTTP correlation，并可校验 Go endpoint。
fn only_http(harness: &Harness, suffix: Option<&str>) -> Correlation {
    let mut correlations = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req }
                if suffix.is_none_or(|expected| req.url.ends_with(expected)) =>
            {
                Some(*corr)
            }
            _ => None,
        });
    let corr = correlations.next().expect("expected one HTTP effect");
    assert!(
        correlations.next().is_none(),
        "expected one HTTP effect only"
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
        .expect("expected one message cache Persist");
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

/// 解出当前批次所有 Emit，验证 older 不借用 timeline 领域广播。
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

/// 构造 latest 依赖页，先附着 anchor 所在窗口再发 older command。
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
    let http = only_http(harness, Some("/posts/getLatestPost"));
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

/// 构造顺序稳定的 Go latest rows，首条成为 older anchor。
fn latest_posts() -> Bytes {
    let posts = (1..=20)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporaryId": format!("mv3-g11b-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11b-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simpleMessage": format!("latest-{index}"),
                "viewers": ["all"],
                "eventSeq": index,
                "createAt": 1_700_001_200_000_i64 + i64::from(index),
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts})).unwrap())
}

/// 构造 latest attached window 的 durable rows，确保 older anchor 来自本地事实。
fn latest_durable_rows() -> Bytes {
    let rows = (1..=20)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporary_id": format!("mv3-g11b-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11b-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simple_message": format!("latest-{index}"),
                "viewers": ["all"],
                "create_at": 1_700_001_200_000_i64 + i64::from(index),
                "event_seq": index,
                "send_status": "sent",
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// 构造 older authority response；内容故意与 durable rows 不同，防止 HTTP 旁路成功。
fn older_posts() -> Bytes {
    let posts = (1..=3)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporaryId": format!("mv3-g11b-older-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11b-author",
                "type": "TEXT",
                "message": format!("http-older-{index}"),
                "simpleMessage": format!("http-older-{index}"),
                "viewers": ["all"],
                "eventSeq": 100 + index,
                "createAt": 1_700_001_100_000_i64 + i64::from(index),
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {"posts": posts, "hasMore": true}
        }))
        .unwrap(),
    )
}

/// 构造一条 durable snake_case row，模拟 exact Scan 的真实单行回报。
fn durable_row(index: i64) -> Bytes {
    let row = json!({
        "id": format!("srvfix{index:020x}"),
        "temporary_id": format!("mv3-g11b-older-{index}"),
        "channel_id": cid(CHANNEL),
        "user_id": "mv3-g11b-author",
        "type": "TEXT",
        "message": format!("durable-older-{index}"),
        "simple_message": format!("durable-older-{index}"),
        "viewers": ["all"],
        "create_at": 1_700_001_100_000_i64 + index,
        "event_seq": 100 + index,
        "send_status": "sent",
    });
    Bytes::from(serde_json::to_vec(&json!([row])).unwrap())
}

/// 构造 canonical older command，只发送 snake_case Rust 载荷。
fn older_command(page_size: u64) -> helix_core::Tick {
    command_json(
        "im_load_older_context",
        json!({
            "channel_id": cid(CHANNEL),
            "anchor": {
                "postId": "srvfix00000000000000000001",
                "createAt": 1_700_001_200_001_i64,
            },
            "anchor_post_id": "srvfix00000000000000000001",
            "page_size": page_size,
            "req_id": REQUEST_ID,
        }),
    )
}

/// G11b 成功链严格为 HTTP → Persist → 每条 authority identity 的 exact Scan → 唯一结果。
#[test]
fn g11b_older_waits_for_durable_scan_and_returns_query_result() {
    let mut harness = scoped_harness();
    attach_latest_window(&mut harness);
    harness.step(older_command(20), 5);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    let request_body = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if *corr == http => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<Value>(body.as_ref()).ok()),
            _ => None,
        })
        .expect("older HTTP body");
    assert_eq!(request_body["direction"], "older");
    assert_eq!(request_body["pageSize"], 20);
    assert!(emitted_events(&harness).is_empty(), "HTTP is not terminal");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(older_posts())),
        6,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "cache Persist is not terminal"
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 7);
    assert!(
        emitted_events(&harness).is_empty(),
        "PersistOk is not terminal"
    );
    let mut readback = only_message_scan(&harness);
    for index in 1_i64..=3 {
        harness.step(
            port_reply_with_body(readback.raw(), durable_row(index)),
            7 + index as u64,
        );
        if index < 3 {
            readback = only_message_scan(&harness);
        }
    }
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "one reqId must have one terminal result");
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert_eq!(events[0]["data"]["body"]["reqId"], REQUEST_ID);
    assert_eq!(
        events[0]["data"]["body"]["messages"][0]["text"],
        "durable-older-1"
    );
    assert_eq!(
        events[0]["data"]["body"]["messages"][2]["text"],
        "durable-older-3"
    );
    assert_eq!(events[0]["data"]["body"]["hasOlder"], true);
    assert_eq!(events[0]["data"]["body"]["hasNewer"], false);
    assert!(
        events[0]["data"]["body"].get("generation").is_none(),
        "G11b terminal must not expose Timeline generation"
    );
    let encoded = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "postReceived",
        "postIncrement",
        "unreadCount",
        "cursor",
        "im:timeline:update",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "forbidden older output: {forbidden}"
        );
    }
}

/// 最新频道 Scan 不能冒充 older authority 页；否则客户端会收到当前窗口并重试同一 anchor。
#[test]
fn g11b_latest_channel_scan_cannot_satisfy_older_authority_page() {
    let mut harness = scoped_harness();
    attach_latest_window(&mut harness);
    harness.step(older_command(20), 5);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    harness.step(
        port_reply_with_body(http.raw(), enveloped(older_posts())),
        6,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 7);
    let readback = only_message_scan(&harness);
    harness.step(
        port_reply_with_body(readback.raw(), latest_durable_rows()),
        8,
    );
    let events = emitted_events(&harness);
    let terminal = events
        .iter()
        .filter(|event| event["data"]["req_id"] == REQUEST_ID)
        .collect::<Vec<_>>();
    assert_eq!(terminal.len(), 1, "older request must settle once");
    assert!(
        terminal[0]["data"].get("error").is_some(),
        "latest channel rows must fail exact authority readback"
    );
    assert!(
        terminal[0]["data"].get("body").is_none()
            || terminal[0]["data"]["body"].get("messages").is_none(),
        "stale latest rows must never become the older success page"
    );
}

/// G11b 页长接受 60，超过上限在任何 storage/http effect 前拒绝。
#[test]
fn g11b_page_size_caps_before_io() {
    let mut harness = scoped_harness();
    attach_latest_window(&mut harness);
    harness.step(older_command(60), 5);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    let body = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if *corr == http => req
                .body
                .as_ref()
                .and_then(|bytes| serde_json::from_slice::<Value>(bytes.as_ref()).ok()),
            _ => None,
        })
        .expect("page size 60 HTTP body");
    assert_eq!(body["pageSize"], 60);
    let bad = command_json(
        "im_load_older_context",
        json!({
            "channel_id": cid(CHANNEL),
            "anchor_post_id": "srvfix00000000000000000001",
            "page_size": 61,
            "req_id": "req-mv3-g11b-bad",
        }),
    );
    harness.sink.clear();
    harness
        .module
        .handle(&bad, 6, &mut harness.sink)
        .expect_err("61 must reject");
    assert!(
        harness.sink.as_slice().is_empty(),
        "invalid page emitted I/O"
    );
}

/// 锚点不需要 attached window；缺 coverage 时直接向 Go 发起一次请求。
#[test]
fn g11b_anchor_miss_starts_one_authority_io() {
    let mut harness = scoped_harness();
    let command = command_json(
        "im_load_older_context",
        json!({
            "channel_id": cid(CHANNEL),
            "anchor": {
                "postId": "srvfix00000000000000000099",
                "createAt": 1_700_001_200_099_i64,
            },
            "anchor_post_id": "srvfix00000000000000000099",
            "page_size": 20,
            "req_id": REQUEST_ID,
        }),
    );
    harness.step(command, 1);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    let body = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if *corr == http => req
                .body
                .as_ref()
                .and_then(|bytes| serde_json::from_slice::<Value>(bytes.as_ref()).ok()),
            _ => None,
        })
        .expect("anchor miss authority body");
    assert_eq!(body["direction"], "older");
    assert_eq!(body["anchor"]["postId"], "srvfix00000000000000000099");
}

/// HTTP 失败只结算关联 error，且不产生 timeline 广播或第二 terminal。
#[test]
fn g11b_http_failure_preserves_old_window() {
    let mut harness = scoped_harness();
    attach_latest_window(&mut harness);
    harness.step(older_command(20), 5);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    harness
        .module
        .handle(
            &helix_core::Tick::PortReply {
                corr: http,
                outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Http(500)),
            },
            6,
            &mut harness.sink,
        )
        .expect("http failure is a terminal result");
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert!(events[0]["data"].get("error").is_some());
}

/// 非空 authority 页被 viewer 全部过滤时，不能伪装成带 hasOlder=true 的成功空页。
#[test]
fn g11b_non_empty_filtered_page_fails_closed_instead_of_retryable_empty_success() {
    let mut harness = scoped_harness();
    attach_latest_window(&mut harness);
    harness.step(older_command(20), 5);
    let http = only_http(&harness, Some("/posts/getPostsAfterIndex"));
    let hidden = json!({
        "status": "SUCCESS",
        "data": {
            "posts": [{
                "id": "srvfix00000000000000000021",
                "temporaryId": "mv3-g11b-hidden",
                "channelId": cid(CHANNEL),
                "userId": "g11b-author",
                "type": "TEXT",
                "message": "hidden older",
                "simpleMessage": "hidden older",
                "viewers": ["g11b-other"],
                "eventSeq": 121,
                "createAt": 1_700_001_100_021_i64
            }],
            "hasMore": true
        }
    });
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&hidden).unwrap())),
        ),
        6,
    );
    assert!(emitted_events(&harness).is_empty(), "HTTP is not terminal");
    let readback = only_message_scan(&harness);
    harness.step(
        port_reply_with_body(readback.raw(), Bytes::from_static(b"[]")),
        7,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "filtered page must settle exactly once");
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert!(
        events[0]["data"].get("error").is_some(),
        "non-empty authority + empty filtered rows must fail, not retry"
    );
    assert_ne!(
        events[0]["data"]["body"]["messages"],
        json!([]),
        "must not emit messages=[] success with hasOlder=true"
    );
}
