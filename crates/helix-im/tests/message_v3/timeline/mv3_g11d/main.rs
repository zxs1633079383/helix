//! MV3-G11d：locate context 必须经 HTTP → Persist → exact durable readback 后返回唯一查询结果。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::Correlation;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL: u64 = 0x11d;
const OTHER_CHANNEL: u64 = 0x11e;
const WINDOW: &str = "mv3-g11d-window";
const REQUEST_ID: &str = "req-mv3-g11d-001";

/// 为 locate 回放注入真实 runtime identity 与指定频道 scope，保持正例可进入渲染闸门。
fn scoped_harness(channel_ids: &[u64]) -> Harness {
    let config = helix_im::module::ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        auth_user_id: "g11d-viewer".to_string(),
        company_id: "g11d-company".to_string(),
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
    for &id in channel_ids {
        harness.module.register_channel(common::channel_id(id), 0);
        harness.seed_channel_scope(&common::cid(id), "g11d-company", "g11d-viewer");
    }
    harness.sink.clear();
    harness.effect_log.clear();
    harness
}

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

/// 从当前批次提取唯一 locate exact message Scan correlation 与 durable identity。
fn only_exact_message_scan(harness: &Harness) -> (Correlation, String) {
    let mut matches = harness.sink.as_slice().iter().filter_map(|effect| {
        let Effect::Persist { corr, ops } = effect else {
            return None;
        };
        ops.iter().find_map(|op| {
            let StorageOp::Scan(spec) = op else {
                return None;
            };
            if spec.table != "message" || spec.limit != Some(1) {
                return None;
            }
            match &spec.filter {
                Some((column, SqlValue::Text(key)))
                    if *column == "temporary_id" || *column == "id" =>
                {
                    Some((*corr, key.clone()))
                }
                _ => None,
            }
        })
    });
    let match_item = matches.next().expect("expected one exact message Scan");
    assert!(
        matches.next().is_none(),
        "expected one exact message Scan only"
    );
    match_item
}

/// 从当前批次提取唯一 message Scan correlation，用于附着 latest 窗口。
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

/// 先以真实 latest local-first 链附着一个可定位的窗口，并保留成功投影断言。
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

/// 构造缺失 temporaryId、只能以 server id 落库的 authority locate 页。
fn id_fallback_locate_posts() -> Bytes {
    let posts = (1..=3)
        .map(|index| {
            json!({
                "id": format!("mv3-g11d-id-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11d-author",
                "type": "TEXT",
                "message": format!("http-id-fallback-{index}"),
                "viewers": ["all"],
                "createAt": 1_700_001_500_000_i64 + i64::from(index),
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "posts": posts,
                "targetIndex": 1,
                "hasOlder": false,
                "hasNewer": false,
            }
        }))
        .unwrap(),
    )
}

/// 构造以 id 查询但同时带另一个 temporary_id 的 durable rows，验证实际列优先。
fn id_fallback_durable_rows() -> Value {
    Value::Array(
        (1..=3)
            .map(|index| {
                json!({
                    "id": format!("mv3-g11d-id-{index}"),
                    "temporary_id": format!("durable-temp-{index}"),
                    "channel_id": cid(CHANNEL),
                    "user_id": "mv3-g11d-author",
                    "type": "TEXT",
                    "message": format!("durable-id-fallback-{index}"),
                    "viewers": ["all"],
                    "create_at": 1_700_001_500_000_i64 + i64::from(index),
                    "send_status": "sent"
                })
            })
            .collect(),
    )
}

/// 构造读回的 durable rows，文本与 HTTP 故意不同以证明结果来自存储事实。
fn durable_rows_value() -> Value {
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
    Value::Array(rows)
}

/// 从 durable fixture 中按 authority identity 返回一条精确读回记录。
fn single_durable_row(rows: &Value, temporary_id: &str) -> Bytes {
    single_durable_row_by_column(rows, "temporary_id", temporary_id)
}

/// 从 durable fixture 中按指定 SQLite 列返回一条精确读回记录。
fn single_durable_row_by_column(rows: &Value, column: &str, value: &str) -> Bytes {
    let row = rows
        .as_array()
        .and_then(|rows| {
            rows.iter().find(|row| {
                row.get(column)
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| candidate == value)
            })
        })
        .cloned()
        .unwrap_or_else(|| panic!("missing durable fixture for {column}={value}"));
    Bytes::from(serde_json::to_vec(&vec![row]).unwrap())
}

/// 构造重复且乱序的第一批 durable readback，验证最终窗口仍按 authority 顺序去重。
fn durable_rows_out_of_order() -> Bytes {
    let rows = durable_rows_value();
    let values = rows.as_array().expect("durable fixture array");
    Bytes::from(
        serde_json::to_vec(&vec![
            values[1].clone(),
            values[0].clone(),
            values[1].clone(),
        ])
        .unwrap(),
    )
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
    locate_command_with(target, REQUEST_ID, "g11d-current")
}

/// 构造可切换 reqId/navigationToken 的 locate 请求，供并发过期回归使用。
fn locate_command_with(target: &str, request_id: &str, navigation_token: &str) -> helix_core::Tick {
    command_json(
        "im_locate_context",
        json!({
            "channel_id": cid(CHANNEL),
            "message_id": target,
            "page_size": 4,
            "req_id": request_id,
            "navigationToken": navigation_token
        }),
    )
}

/// temporaryId 目标必须沿用 HTTP → Persist → exact identity readback 的真实 locate 链路。
#[test]
fn g11d_context_accepts_temporary_id_target() {
    let mut harness = scoped_harness(&[CHANNEL]);
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
    let durable = durable_rows_value();
    let mut readback = only_exact_message_scan(&harness);
    for index in 1..=5 {
        assert_eq!(readback.1, format!("mv3-g11d-{index}"));
        harness.step(
            port_reply_with_body(readback.0.raw(), single_durable_row(&durable, &readback.1)),
            8 + index,
        );
        if index < 5 {
            readback = only_exact_message_scan(&harness);
        }
    }
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "one reqId has one terminal result");
    let body = &events[0]["data"]["body"];
    assert_eq!(body["targetPostId"], "mv3-g11d-3");
    assert_eq!(body["targetIndex"], 2);
    assert_eq!(body["messages"][2]["temporaryId"], "mv3-g11d-3");
}

/// Locate 成功必须是 HTTP → Persist → exact Scan → 一个 QueryMessagesResult，且不广播 timeline。
#[test]
fn g11d_context_waits_for_centered_durable_window() {
    let mut harness = scoped_harness(&[CHANNEL]);
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
    let (readback, identity) = only_exact_message_scan(&harness);
    assert_eq!(identity, "mv3-g11d-1");
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
        .expect("locate exact durable Scan");
    assert_eq!(scan.limit, Some(1));
    assert!(
        scan.order_by.is_empty(),
        "exact identity readback has no order"
    );
    match &scan.filter {
        Some((column, SqlValue::Text(key))) => {
            assert_eq!(*column, "temporary_id");
            assert_eq!(key, "mv3-g11d-1");
        }
        other => panic!("expected temporary_id exact filter, got {other:?}"),
    }
    let durable = durable_rows_value();
    let mut current = (readback, identity);
    for index in 1..=5 {
        assert_eq!(current.1, format!("mv3-g11d-{index}"));
        harness.step(
            port_reply_with_body(current.0.raw(), single_durable_row(&durable, &current.1)),
            8 + index,
        );
        if index < 5 {
            current = only_exact_message_scan(&harness);
        }
    }
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

/// 缺失 temporaryId 时，Locate 必须按实际 durable `id` 主键逐条读回。
#[test]
fn g11d_locate_falls_back_to_id_storage_key() {
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(
        locate_command_with("mv3-g11d-id-2", "req-mv3-g11d-id", "g11d-id"),
        6,
    );
    let http = only_http(&harness, "/posts/postContext");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(id_fallback_locate_posts())),
        7,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let (readback, identity) = only_exact_message_scan(&harness);
    assert_eq!(identity, "mv3-g11d-id-1");
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
        .expect("id fallback exact Scan");
    match &scan.filter {
        Some((column, SqlValue::Text(key))) => {
            assert_eq!(*column, "id");
            assert_eq!(key, "mv3-g11d-id-1");
        }
        other => panic!("expected id exact filter, got {other:?}"),
    }
    let durable = id_fallback_durable_rows();
    let mut current = (readback, identity);
    for index in 1..=3 {
        let expected = format!("mv3-g11d-id-{index}");
        assert_eq!(current.1, expected);
        harness.step(
            port_reply_with_body(
                current.0.raw(),
                single_durable_row_by_column(&durable, "id", &current.1),
            ),
            8 + index,
        );
        if index < 3 {
            current = only_exact_message_scan(&harness);
        }
    }
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], "req-mv3-g11d-id");
    assert_eq!(events[0]["data"]["body"]["targetIndex"], 1);
    assert_eq!(
        events[0]["data"]["body"]["messages"][1]["id"],
        "mv3-g11d-id-2"
    );
}

/// 重复/乱序 durable 回报不得重复发布结果，也不得改变 authority 的窗口顺序。
#[test]
fn g11d_locate_deduplicates_out_of_order_readback() {
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(locate_posts())),
        7,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let (first, identity) = only_exact_message_scan(&harness);
    assert_eq!(identity, "mv3-g11d-1");
    harness.step(
        port_reply_with_body(first.raw(), durable_rows_out_of_order()),
        9,
    );
    let (mut readback, mut next_identity) = only_exact_message_scan(&harness);
    assert_eq!(next_identity, "mv3-g11d-3");
    let durable = durable_rows_value();
    for index in 3..=5 {
        assert_eq!(next_identity, format!("mv3-g11d-{index}"));
        harness.step(
            port_reply_with_body(readback.raw(), single_durable_row(&durable, &next_identity)),
            9 + index,
        );
        if index < 5 {
            (readback, next_identity) = only_exact_message_scan(&harness);
        }
    }
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1, "duplicate readback still has one terminal");
    let messages = events[0]["data"]["body"]["messages"]
        .as_array()
        .expect("locate messages");
    assert_eq!(messages.len(), 4);
    let ids = messages
        .iter()
        .map(|message| message["id"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            "srvfix00000000000000000001",
            "srvfix00000000000000000002",
            "srvfix00000000000000000003",
            "srvfix00000000000000000004",
        ]
    );
}

/// Locate authority 缺失目标时必须失败保旧，不能用其它 rows 猜测 targetIndex。
#[test]
fn g11d_locate_missing_target_fails_before_persist() {
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(
        locate_command_with("mv3-g11d-not-found", "req-missing", "missing"),
        6,
    );
    let http = only_http(&harness, "/posts/postContext");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(locate_posts())),
        7,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert!(events[0]["data"].get("error").is_some());
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. })));
}

/// Locate authority 任何跨频道 row 都必须失败，不能混入当前频道窗口。
#[test]
fn g11d_locate_cross_channel_fails_before_persist() {
    let mut harness = scoped_harness(&[CHANNEL, OTHER_CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    let mut cross_channel: Value = serde_json::from_slice(locate_posts().as_ref()).unwrap();
    cross_channel["data"]["posts"][0]["channelId"] = json!(cid(OTHER_CHANNEL));
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&cross_channel).unwrap())),
        ),
        7,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert!(events[0]["data"].get("error").is_some());
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. })));
}

/// Locate authority 缺失 temporaryId 与 id 时必须 fail closed，不产生无身份缓存写入。
#[test]
fn g11d_locate_missing_identity_fails_before_persist() {
    // 缺的是 authority 消息的 temporaryId/id，不是 RuntimeAuth；避免 scope 错误掩盖本负例。
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(locate_command(3), 6);
    let http = only_http(&harness, "/posts/postContext");
    let mut missing_identity: Value = serde_json::from_slice(locate_posts().as_ref()).unwrap();
    let target = missing_identity["data"]["posts"][2]
        .as_object_mut()
        .expect("target object");
    target.remove("temporaryId");
    target.remove("id");
    harness.step(
        port_reply_with_body(
            http.raw(),
            enveloped(Bytes::from(serde_json::to_vec(&missing_identity).unwrap())),
        ),
        7,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert!(events[0]["data"].get("error").is_some());
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Persist { .. })));
}

/// locate 等待 authority 时并发 latest 刷新不得让同 reqId 的真实回包失效。
#[test]
fn g11d_latest_refresh_does_not_cancel_inflight_locate_request() {
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(
        locate_command_with(
            "srvfix00000000000000000003",
            "req-after-refresh",
            "token-after-refresh",
        ),
        6,
    );
    let locate_http = only_http(&harness, "/posts/postContext");

    // 模拟 UI 重进频道时发出的 latest refresh；它会替换同频道窗口但不拥有 locate reqId。
    attach_latest_window(&mut harness);
    harness.step(
        port_reply_with_body(locate_http.raw(), enveloped(locate_posts())),
        8,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 9);
    let durable = durable_rows_value();
    let mut readback = only_exact_message_scan(&harness);
    for index in 1..=5 {
        harness.step(
            port_reply_with_body(readback.0.raw(), single_durable_row(&durable, &readback.1)),
            9 + index,
        );
        if index < 5 {
            readback = only_exact_message_scan(&harness);
        }
    }
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], "req-after-refresh");
    assert!(events[0]["data"].get("error").is_none());
    assert_eq!(
        events[0]["data"]["body"]["targetPostId"],
        "srvfix00000000000000000003"
    );
    assert_eq!(events[0]["data"]["body"]["targetIndex"], 2);
}

/// 已完成 Locate coverage 再次定位同一目标时只读本地 coverage，不重复请求 Go。
#[test]
fn g11d_repeated_locate_coverage_hit_avoids_go_http() {
    let mut harness = scoped_harness(&[CHANNEL]);
    attach_latest_window(&mut harness);
    harness.step(
        locate_command_with("srvfix00000000000000000003", "req-first", "token-first"),
        6,
    );
    let http = only_http(&harness, "/posts/postContext");
    harness.step(
        port_reply_with_body(http.raw(), enveloped(locate_posts())),
        7,
    );
    let cache = only_cache_write(&harness);
    harness.step(port_reply_with_body(cache.raw(), Bytes::new()), 8);
    let durable = durable_rows_value();
    let mut readback = only_exact_message_scan(&harness);
    for index in 1..=5 {
        assert_eq!(readback.1, format!("mv3-g11d-{index}"));
        harness.step(
            port_reply_with_body(readback.0.raw(), single_durable_row(&durable, &readback.1)),
            8 + index,
        );
        if index < 5 {
            readback = only_exact_message_scan(&harness);
        }
    }
    assert_eq!(emitted_events(&harness).len(), 1);

    harness.step(
        locate_command_with(
            "srvfix00000000000000000003",
            "req-coverage",
            "token-coverage",
        ),
        20,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["req_id"], "req-coverage");
    assert_eq!(events[0]["data"]["body"]["targetIndex"], 2);
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. } | Effect::Persist { .. })));
}

/// 撤回目标在 authority 阶段 fail closed，只结算 error 并保留旧窗口。
#[test]
fn g11d_revoked_target_has_correlated_error_without_persist() {
    let mut harness = scoped_harness(&[CHANNEL]);
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
    let mut harness = scoped_harness(&[CHANNEL]);
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
    let (mut readback, mut identity) = only_exact_message_scan(&harness);
    assert_eq!(identity, "mv3-g11d-visible-1");
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
    for expected in [
        "mv3-g11d-visible-1",
        "mv3-g11d-target-3",
        "mv3-g11d-visible-4",
    ] {
        assert_eq!(identity, expected);
        harness.step(
            port_reply_with_body(readback.raw(), single_durable_row(&durable, &identity)),
            9,
        );
        if expected != "mv3-g11d-visible-4" {
            (readback, identity) = only_exact_message_scan(&harness);
        }
    }
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
