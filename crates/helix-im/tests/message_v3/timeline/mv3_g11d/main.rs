//! MV3-G11d：locate context 必须经 HTTP → Persist → bounded Scan 后返回唯一查询结果。

use bytes::Bytes;
use helix_core::effect::{Effect, ScanOrder, StorageOp};
use helix_core::Correlation;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL: u64 = 0x11d;
const WINDOW: &str = "mv3-g11d-window";
const REQUEST_ID: &str = "req-mv3-g11d-001";

/// 从当前批次提取唯一 HTTP correlation，并校验 locate authority endpoint。
fn only_http(harness: &Harness, suffix: &str) -> Correlation {
    let mut correlations = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
            _ => None,
        });
    let corr = correlations
        .next()
        .expect("expected one locate HTTP effect");
    assert!(
        correlations.next().is_none(),
        "expected one locate HTTP effect only"
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
        .expect("expected one locate cache Persist");
    assert!(
        correlations.next().is_none(),
        "expected one cache Persist only"
    );
    corr
}

/// 从当前批次提取唯一 locate bounded message Scan correlation。
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

/// 解出当前批次所有 Emit，确保 locate 不发布第二条 timeline 领域广播。
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

/// 先以真实 latest local-first 链附着一个可定位的窗口。
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
    let http = only_http(harness, "/posts/getLatestPost");
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
        "latest attaches one window"
    );
}

/// 构造一页 locate authority rows，目标位于中间并返回双向 opaque cursor。
fn locate_posts() -> Bytes {
    let posts = (1..=5)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporaryId": format!("mv3-g11d-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11d-author",
                "type": "TEXT",
                "message": format!("http-locate-{index}"),
                "simpleMessage": format!("http-locate-{index}"),
                "viewers": ["all"],
                "eventSeq": index,
                "createAt": 1_700_001_400_000_i64 + i64::from(index),
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "posts": posts,
                "targetIndex": 2,
                "hasOlder": true,
                "hasNewer": true,
                "olderCursor": "opaque-older-1",
                "newerCursor": "opaque-newer-1"
            }
        }))
        .unwrap(),
    )
}

/// 构造读回的 durable rows，文本与 HTTP 故意不同以证明结果来自存储事实。
fn durable_rows() -> Bytes {
    let rows = (1..=5)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporary_id": format!("mv3-g11d-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11d-author",
                "type": "TEXT",
                "message": format!("durable-locate-{index}"),
                "simple_message": format!("durable-locate-{index}"),
                "viewers": ["all"],
                "create_at": 1_700_001_400_000_i64 + i64::from(index),
                "event_seq": index,
                "send_status": "sent"
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// 构造 latest 的 Go rows，确保 locate 前有 attached timeline。
fn latest_posts() -> Bytes {
    let posts = (1..=6)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporaryId": format!("latest-g11d-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11d-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simpleMessage": format!("latest-{index}"),
                "viewers": ["all"],
                "eventSeq": index,
                "createAt": 1_700_001_300_000_i64 + i64::from(index)
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts})).unwrap())
}

/// 构造 latest 的 durable rows，供 attached window 的本地事实读回。
fn latest_durable_rows() -> Bytes {
    let rows = (1..=6)
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporary_id": format!("latest-g11d-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11d-author",
                "type": "TEXT",
                "message": format!("latest-{index}"),
                "simple_message": format!("latest-{index}"),
                "viewers": ["all"],
                "create_at": 1_700_001_300_000_i64 + i64::from(index),
                "event_seq": index,
                "send_status": "sent"
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// 构造 canonical im_locate_context 请求。
fn locate_command(target: u64) -> helix_core::Tick {
    locate_command_for_target(&format!("srvfix{target:020x}"))
}

/// 构造以 temporaryId 作为目标身份的 locate 请求，覆盖移动端真实调用形态。
fn locate_command_for_target(target: &str) -> helix_core::Tick {
    command_json(
        "im_locate_context",
        json!({
            "channel_id": cid(CHANNEL),
            "message_id": target,
            "page_size": 4,
            "req_id": REQUEST_ID,
            "navigationToken": "g11d-current"
        }),
    )
}

/// temporaryId 目标必须沿用 HTTP → Persist → bounded Scan 的真实 locate 链路。
#[test]
fn g11d_context_accepts_temporary_id_target() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(locate_command_for_target("mv3-g11d-3"), 6);
    let http = only_http(&harness, "/posts/postContext");
    assert_eq!(
        harness.last_http_body().expect("locate HTTP body")["postId"],
        "mv3-g11d-3"
    );
    harness.step(
        port_reply_with_body(http.raw(), enveloped(locate_posts())),
        7,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "HTTP cannot settle locate"
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    assert!(
        emitted_events(&harness).is_empty(),
        "PersistOk cannot settle locate"
    );
    let readback = only_message_scan(&harness);
    harness.step(port_reply_with_body(readback.raw(), durable_rows()), 9);
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "one reqId has one terminal result");
    let body = &events[0]["data"]["body"];
    assert_eq!(body["targetPostId"], "mv3-g11d-3");
    assert_eq!(body["targetIndex"], 2);
    assert_eq!(body["messages"][2]["temporaryId"], "mv3-g11d-3");
}

/// Locate 成功必须是 HTTP → Persist → Scan → 一个 QueryMessagesResult，且不广播 timeline。
#[test]
fn g11d_context_waits_for_centered_durable_window() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    assert_eq!(
        harness.last_http_body().expect("locate HTTP body")["postId"],
        "srvfix00000000000000000003"
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "HTTP cannot settle locate"
    );
    harness.step(
        port_reply_with_body(http.raw(), enveloped(locate_posts())),
        7,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "Persist cannot settle locate"
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    assert!(
        emitted_events(&harness).is_empty(),
        "PersistOk cannot settle locate"
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
        .expect("locate bounded Scan");
    assert_eq!(scan.limit, Some(5));
    assert_eq!(
        scan.order_by,
        &[ScanOrder::asc("create_at"), ScanOrder::asc("temporary_id")]
    );
    harness.step(port_reply_with_body(readback.raw(), durable_rows()), 9);
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "one reqId has one terminal result");
    assert_eq!(events[0]["event"], "im:read:result");
    assert_eq!(events[0]["data"]["req_id"], REQUEST_ID);
    assert_eq!(events[0]["data"]["body"]["reqId"], REQUEST_ID);
    assert_eq!(
        events[0]["data"]["body"]["messages"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
    assert_eq!(
        events[0]["data"]["body"]["messages"][0]["text"],
        "durable-locate-1"
    );
    assert_eq!(
        events[0]["data"]["body"]["messages"][2]["id"],
        "srvfix00000000000000000003"
    );
    assert_eq!(events[0]["data"]["body"]["olderCursor"], "opaque-older-1");
    assert_eq!(events[0]["data"]["body"]["newerCursor"], "opaque-newer-1");
    assert!(
        events[0]["data"]["body"].get("generation").is_none(),
        "G11d terminal must not expose Timeline generation"
    );
    let encoded = serde_json::to_string(&events).unwrap();
    for forbidden in [
        "im:timeline:located",
        "im:timeline:update",
        "postReceived",
        "postIncrement",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "forbidden locate output: {forbidden}"
        );
    }
}

/// 撤回目标在 authority 阶段 fail closed，只结算 error 并保留旧窗口。
#[test]
fn g11d_revoked_target_has_correlated_error_without_persist() {
    let mut harness =
        Harness::new_with_store_mode(&[CHANNEL], helix_im::query::LocalStoreMode::Session);
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    let revoked = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "posts": [{
                "id": "srvfix00000000000000000003",
                "temporaryId": "mv3-g11d-3",
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11d-author",
                "type": "TEXT",
                "message": "revoked",
                "viewers": ["all"],
                "revoke": true,
                "createAt": 1_700_001_400_003_i64
            }],
            "targetIndex": 0,
            "hasOlder": true,
            "hasNewer": true
        }
    });
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&revoked).unwrap())),
        ),
        7,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:read:result");
    assert!(events[0]["data"].get("error").is_some());
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. })));
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("im:timeline:"));
}

/// 隐藏前置消息被过滤后，locate 的 targetIndex 必须按可见 durable 窗口重算。
#[test]
fn g11d_hidden_predecessor_reindexes_visible_target() {
    let mut harness =
        Harness::new_with_identity(&[CHANNEL], "g11d-viewer", "g11d-company", "", "", "");
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    let authority = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "posts": [
                {
                    "id": "srvfix00000000000000000001",
                    "temporaryId": "mv3-g11d-visible-1",
                    "channelId": cid(CHANNEL),
                    "userId": "g11d-author",
                    "type": "TEXT",
                    "message": "visible before",
                    "viewers": ["all"],
                    "createAt": 1_700_001_400_001_i64
                },
                {
                    "id": "srvfix00000000000000000002",
                    "temporaryId": "mv3-g11d-hidden-2",
                    "channelId": cid(CHANNEL),
                    "userId": "g11d-author",
                    "type": "TEXT",
                    "message": "hidden before",
                    "viewers": ["g11d-other"],
                    "createAt": 1_700_001_400_002_i64
                },
                {
                    "id": "srvfix00000000000000000003",
                    "temporaryId": "mv3-g11d-target-3",
                    "channelId": cid(CHANNEL),
                    "userId": "g11d-author",
                    "type": "TEXT",
                    "message": "target",
                    "viewers": ["all"],
                    "createAt": 1_700_001_400_003_i64
                },
                {
                    "id": "srvfix00000000000000000004",
                    "temporaryId": "mv3-g11d-visible-4",
                    "channelId": cid(CHANNEL),
                    "userId": "g11d-author",
                    "type": "TEXT",
                    "message": "visible after",
                    "viewers": ["all"],
                    "createAt": 1_700_001_400_004_i64
                }
            ],
            "targetIndex": 2,
            "hasOlder": true,
            "hasNewer": true
        }
    });
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&authority).unwrap())),
        ),
        7,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let readback = only_message_scan(&harness);
    let durable = serde_json::json!([
        {
            "id": "srvfix00000000000000000001",
            "temporary_id": "mv3-g11d-visible-1",
            "channel_id": cid(CHANNEL),
            "user_id": "g11d-author",
            "type": "TEXT",
            "message": "durable before",
            "viewers": ["all"],
            "create_at": 1_700_001_400_001_i64,
            "send_status": "sent"
        },
        {
            "id": "srvfix00000000000000000003",
            "temporary_id": "mv3-g11d-target-3",
            "channel_id": cid(CHANNEL),
            "user_id": "g11d-author",
            "type": "TEXT",
            "message": "durable target",
            "viewers": ["all"],
            "create_at": 1_700_001_400_003_i64,
            "send_status": "sent"
        },
        {
            "id": "srvfix00000000000000000004",
            "temporary_id": "mv3-g11d-visible-4",
            "channel_id": cid(CHANNEL),
            "user_id": "g11d-author",
            "type": "TEXT",
            "message": "durable after",
            "viewers": ["all"],
            "create_at": 1_700_001_400_004_i64,
            "send_status": "sent"
        }
    ]);
    harness.step(
        port_reply_with_body(
            readback.raw(),
            Bytes::from(serde_json::to_vec(&durable).unwrap()),
        ),
        9,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    let body = &events[0]["data"]["body"];
    assert_eq!(
        body["targetIndex"], 1,
        "hidden predecessor shifts target index"
    );
    assert_eq!(body["messages"][1]["id"], "srvfix00000000000000000003");
    assert_eq!(body["hasOlder"], true);
    assert_eq!(body["hasNewer"], true);
}
