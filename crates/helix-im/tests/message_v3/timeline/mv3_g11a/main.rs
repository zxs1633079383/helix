//! MV3-G11a：latest Timeline 必须经过 durable cache Persist 与同规格 Scan read-back。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::{Correlation, Module};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL: u64 = 0x11a;
const WINDOW: &str = "mv3-g11a-latest";

/// 为 latest 回放注入真实 runtime identity 与 channel scope，避免正例被渲染闸门误判为越权。
fn scoped_harness() -> Harness {
    let config = helix_im::module::ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        auth_user_id: "g11a-viewer".to_string(),
        company_id: "g11a-company".to_string(),
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
    harness.seed_channel_scope(&common::cid(CHANNEL), "g11a-company", "g11a-viewer");
    harness.sink.clear();
    harness.effect_log.clear();
    harness
}

/// 从当前批次取唯一 HTTP correlation，防止隐藏的额外网络请求。
fn only_http(harness: &Harness) -> Correlation {
    let mut correlations = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        });
    let corr = correlations.next().expect("expected one HTTP effect");
    assert!(
        correlations.next().is_none(),
        "latest query must issue at most one HTTP effect"
    );
    corr
}

/// 从当前批次取唯一 message Scan correlation。
fn only_scan(harness: &Harness) -> Correlation {
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
    let corr = correlations
        .next()
        .expect("expected one message Scan effect");
    assert!(
        correlations.next().is_none(),
        "latest query must issue one message Scan per stage"
    );
    corr
}

/// 从当前批次取唯一 message cache Persist correlation。
fn only_cache_write(harness: &Harness) -> Correlation {
    let mut correlations = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(
                        op,
                        StorageOp::BatchUpsert(spec) if spec.table == "message"
                    )
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        });
    let corr = correlations.next().expect("expected message cache Persist");
    assert!(
        correlations.next().is_none(),
        "latest query must issue one cache Persist"
    );
    corr
}

/// 解出当前批次全部 producer 事件，不把 HTTP 或 Persist 当成终态。
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

/// 构造真实 Go latest Post 数组，远端消息内容刻意与 read-back 不同。
fn latest_posts(marker: &str) -> Bytes {
    let posts = (1..=20)
        .rev()
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporaryId": format!("mv3-g11a-{index}"),
                "channelId": cid(CHANNEL),
                "userId": "mv3-g11a-user",
                "type": "TEXT",
                "message": format!("{marker}-{index}"),
                "simpleMessage": format!("{marker}-{index}"),
                "viewers": ["all"],
                "mentions": [],
                "eventSeq": index,
                "createAt": 1_700_001_100_000_i64 + i64::from(index),
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&json!({"status": "SUCCESS", "data": posts})).unwrap())
}

/// 构造 SQLite message Scan 的 snake_case durable rows。
fn durable_rows(marker: &str) -> Bytes {
    let rows = (1..=20)
        .rev()
        .map(|index| {
            json!({
                "id": format!("srvfix{index:020x}"),
                "temporary_id": format!("mv3-g11a-{index}"),
                "channel_id": cid(CHANNEL),
                "user_id": "mv3-g11a-user",
                "type": "TEXT",
                "message": format!("{marker}-{index}"),
                "simple_message": format!("{marker}-{index}"),
                "create_at": 1_700_001_100_000_i64 + i64::from(index),
                "send_status": "sent",
                "event_seq": index,
            })
        })
        .collect::<Vec<_>>();
    Bytes::from(serde_json::to_vec(&rows).unwrap())
}

/// 构造 canonical latest query payload。
fn latest_command(page_size: u64) -> helix_core::Tick {
    command_json(
        "im_query_messages_by_channel",
        json!({
            "channel_id": cid(CHANNEL),
            "pageSize": page_size,
            "windowToken": WINDOW,
        }),
    )
}

#[test]
fn g11a_latest_waits_for_durable_readback_and_matches_go_wire() {
    let mut harness = scoped_harness();

    harness.step(latest_command(20), 1);
    let initial_scan = only_scan(&harness);
    assert!(emitted_events(&harness).is_empty());

    harness.step(
        port_reply_with_body(initial_scan.raw(), Bytes::from_static(b"[]")),
        2,
    );
    let http = only_http(&harness);
    let http_request = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if *corr == http => Some(req),
            _ => None,
        })
        .expect("latest HTTP request must be present");
    assert_eq!(http_request.method, "POST");
    assert!(http_request.url.ends_with("/posts/getLatestPost"));
    let body: Value = serde_json::from_slice(
        http_request
            .body
            .as_ref()
            .expect("latest HTTP body")
            .as_ref(),
    )
    .expect("latest HTTP body JSON");
    assert_eq!(body["channelId"], cid(CHANNEL));
    assert_eq!(body["timestamp"], 0);
    assert_eq!(body["cursorVersion"], 1);
    assert_eq!(body["pageSize"], 20);

    harness.step(
        port_reply_with_body(http.raw(), enveloped(latest_posts("http"))),
        3,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "HTTP JSON is not terminal"
    );
    let cache = only_cache_write(&harness);

    harness.step(
        port_reply_with_body(cache.raw(), Bytes::from_static(b"[]")),
        4,
    );
    assert!(
        emitted_events(&harness).is_empty(),
        "cache Persist is not terminal"
    );
    let readback = only_scan(&harness);

    harness.step(
        port_reply_with_body(readback.raw(), durable_rows("durable")),
        5,
    );
    let events = emitted_events(&harness);
    assert_eq!(
        events.len(),
        1,
        "durable read-back must emit exactly one terminal event"
    );
    assert_eq!(events[0]["event"], "im:timeline:ready");
    assert_eq!(events[0]["data"]["state"], "ready");
    assert_eq!(events[0]["data"]["messages"][0]["message"], "durable-1");
    assert_eq!(events[0]["data"]["messages"][19]["message"], "durable-20");
    assert!(!serde_json::to_string(&events)
        .unwrap()
        .contains("postReceived"));
}

#[test]
fn g11a_page_size_over_sixty_fails_before_storage_or_http() {
    let mut harness = scoped_harness();
    let tick = latest_command(61);
    let result = harness.module.handle(&tick, 1, &mut harness.sink);
    assert!(result.is_err(), "pageSize=61 must fail before I/O");
    assert!(
        harness.sink.as_slice().is_empty(),
        "invalid pageSize emitted an effect"
    );
}

#[test]
fn g11a_late_generation_reply_is_ignored() {
    let mut harness = scoped_harness();
    harness.step(latest_command(20), 1);
    let first_scan = only_scan(&harness);

    harness.step(latest_command(20), 2);
    let second_scan = only_scan(&harness);

    harness.step(
        port_reply_with_body(first_scan.raw(), Bytes::from_static(b"[]")),
        3,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "late first generation must not start HTTP or emit"
    );

    harness.step(
        port_reply_with_body(second_scan.raw(), Bytes::from_static(b"[]")),
        4,
    );
    assert!(
        harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Http { .. })),
        "current generation still falls back to HTTP"
    );
    assert!(emitted_events(&harness).is_empty());
}
