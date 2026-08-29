// 回归锚: Spec 047/T060-T061，G-11a 未缓存定位与乱序 token。

use bytes::Bytes;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Correlation;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../g10c/main.rs"]
mod base;

const CHANNEL_ID: &str = "chfixx0000000000000000001c";
const CREATED_AT: i64 = 1_700_001_300_000;

type TestResult<T> = Result<T, String>;

/// 记录当前 token 与 stale token 的两次 terminal event。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 发起一个带稳定 token 的未缓存 locate 并返回 HTTP correlation。
fn start_locate(
    replay: &mut base::Replay,
    target_index: u32,
    token: &str,
) -> TestResult<Correlation> {
    replay.command(
        "im_locate_message",
        json!({
            "channel_id": CHANNEL_ID,
            "message_id": base::server_id(target_index),
            "pageSize": 20,
            "req_id": format!("g11a-{token}"),
            "navigationToken": token,
        }),
    )?;
    replay.only_corr("http", Some("/posts/postContext"))
}

/// 完成一个 locate 的 HTTP、cache PersistOk 与逐 identity matching durable readback。
fn settle_locate(
    replay: &mut base::Replay,
    http: Correlation,
    target_index: u32,
    offset_ms: u64,
) -> TestResult<Vec<Value>> {
    replay.reply(
        http,
        PortOutcome::Ok(ReplyBytes(base::http_envelope(locate_body(target_index)?)?)),
        offset_ms,
    )?;
    let cache = replay.only_corr("write", None)?;
    replay.reply(cache, PortOutcome::Ok(ReplyBytes::default()), offset_ms + 1)?;
    let start = target_index.saturating_sub(5).max(1);
    for (offset, index) in (start..=target_index + 5).enumerate() {
        let readback = replay.only_corr("readback", None)?;
        let row = serde_json::to_vec(&vec![base::local_post(index)])
            .map_err(|error| error.to_string())?;
        replay.reply(
            readback,
            PortOutcome::Ok(ReplyBytes(Bytes::from(row))),
            offset_ms + 2 + offset as u64,
        )?;
    }
    replay.events()
}

/// 回放 current token 先完成、stale token 后完成的乱序定位。
fn run_out_of_order() -> TestResult<PersonaTrace> {
    let mut replay = base::Replay::new()?;
    replay.attach_full_window()?;
    let stale_http = start_locate(&mut replay, 8, "g11a-stale")?;
    let current_http = start_locate(&mut replay, 10, "g11a-current")?;
    let mut events = settle_locate(&mut replay, current_http, 10, 10)?;
    events.extend(settle_locate(&mut replay, stale_http, 8, 20)?);
    Ok(PersonaTrace {
        events,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_locate_message", "payload": {
                    "channel_id": CHANNEL_ID,
                    "message_id": base::server_id(8),
                    "navigationToken": "g11a-stale"
                }},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "Command",
                "value": {"name": "im_locate_message", "payload": {
                    "channel_id": CHANNEL_ID,
                    "message_id": base::server_id(10),
                    "navigationToken": "g11a-current"
                }},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": current_http.raw(), "outcome": {"variant": "Ok"}},
                "now_ms": CREATED_AT + 10
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": stale_http.raw(), "outcome": {"variant": "Ok"}},
                "now_ms": CREATED_AT + 20
            }),
        ],
    })
}

/// 构造 target 位于中心的 postContext CommonRes。
fn locate_body(target_index: u32) -> TestResult<Bytes> {
    let start = target_index.saturating_sub(5).max(1);
    let posts = (start..=target_index + 5)
        .map(remote_post)
        .collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": {
            "posts": posts,
            "targetIndex": usize::try_from(target_index - start)
                .map_err(|error| error.to_string())?,
            "hasOlder": true,
            "hasNewer": true
        }
    }))
    .map(Bytes::from)
    .map_err(|error| error.to_string())
}

/// 构造定位 authority 使用的远端 post。
fn remote_post(index: u32) -> Value {
    json!({
        "id": base::server_id(index),
        "temporaryId": format!("tmpfix{index:020x}"),
        "channelId": CHANNEL_ID,
        "userId": "g10c-actor",
        "type": "TEXT",
        "message": format!("G-11a message {index}"),
        "simpleMessage": format!("G-11a message {index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
}

/// G-11a 的 token 生命周期由 Angular 持有；Helix 为每次 locate 返回独立 reqId 结果。
#[test]
fn g11a_returns_independent_req_results_for_out_of_order_tokens() -> TestResult<()> {
    let trace = run_out_of_order()?;
    assert_eq!(trace.events.len(), 2);
    for (event, request_id, target_id) in [
        (&trace.events[0], "g11a-g11a-current", base::server_id(10)),
        (&trace.events[1], "g11a-g11a-stale", base::server_id(8)),
    ] {
        assert_eq!(event["event"], "im:read:result");
        assert_eq!(event["data"]["req_id"], request_id);
        let body = &event["data"]["body"];
        assert_eq!(body["reqId"], request_id);
        assert_eq!(body["operation"], "locate");
        assert_eq!(body["targetPostId"], target_id);
        assert_eq!(body["targetIndex"], 5);
    }
    Ok(())
}

/// G-11a 两次响应都经过独立 PersistOk/readback，且事件不含 renderer activation。
#[test]
fn g11a_keeps_out_of_order_merge_free_of_renderer_directives() -> TestResult<()> {
    let trace = run_out_of_order()?;
    let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "InteractionFrame",
        "Surface",
        "ActivateWindow",
        "directive",
        "mutation",
        "actionCapabilities",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-11a sender 与空 receiver canonical artifact。
#[test]
fn g11a_writes_reviewable_persona_artifacts() -> TestResult<()> {
    let trace = run_out_of_order()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-11a",
        "message_v3_g11a",
        trace.ticks,
        trace.events,
        Vec::new(),
        json!([
            {"table": "post", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "create_at"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
        json!([
            {"table": "message", "columns": ["id", "temporary_id", "channel_id", "user_id", "type", "message", "send_status", "create_at", "event_seq"]}
        ]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
