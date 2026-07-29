// 回归锚: Spec 047/T056-T057，G-10d 离底 anchor 与新消息提示。

use bytes::Bytes;
use helix_core::tick::{PortOutcome, ReplyBytes};
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../g10c/main.rs"]
pub(crate) mod base;

const CHANNEL_ID: &str = "chfixx0000000000000000001c";
const TARGET_ID: &str = "srvfix0000000000000000000a";
const CREATED_AT: i64 = 1_700_001_300_000;

type TestResult<T> = Result<T, String>;

/// 记录 receiver 离底窗口的真实事件和输入 Tick。
struct PersonaTrace {
    events: Vec<Value>,
    ticks: Vec<Value>,
}

/// 通过真实 locate authority 把 attached latest window 切换为稳定 anchor。
pub(crate) fn locate_away_from_bottom(replay: &mut base::Replay) -> TestResult<()> {
    replay.command(
        "im_locate_message",
        json!({
            "channel_id": CHANNEL_ID,
            "message_id": TARGET_ID,
            "pageSize": 20,
            "req_id": "g10d-locate-request",
            "navigationToken": "g10d-navigation",
        }),
    )?;
    let http = replay.only_corr("http", Some("/posts/postContext"))?;
    replay.reply(
        http,
        PortOutcome::Ok(ReplyBytes(base::http_envelope(locate_body()?)?)),
        4,
    )?;
    let cache = replay.only_corr("write", None)?;
    replay.reply(cache, PortOutcome::Ok(ReplyBytes::default()), 5)?;
    let readback = replay.only_corr("readback", None)?;
    let target_row =
        serde_json::to_vec(&vec![base::local_post(10)]).map_err(|error| error.to_string())?;
    replay.reply(
        readback,
        PortOutcome::Ok(ReplyBytes(Bytes::from(target_row))),
        6,
    )?;
    if replay.events()?.len() != 1 {
        return Err("G-10d locate prerequisite must publish one terminal result".to_string());
    }
    Ok(())
}

/// 回放离底窗口收到新消息后的 attached refresh。
fn run_success() -> TestResult<PersonaTrace> {
    let mut replay = base::Replay::new()?;
    replay.attach_full_window()?;
    locate_away_from_bottom(&mut replay)?;
    let send_persist = replay.start_send()?;
    replay.reply(send_persist, PortOutcome::Ok(ReplyBytes::default()), 10)?;
    let refresh = replay.only_corr("scan", None)?;
    replay.reply(
        refresh,
        PortOutcome::Ok(ReplyBytes(base::local_rows_after_append()?)),
        11,
    )?;
    Ok(PersonaTrace {
        events: replay.events()?,
        ticks: vec![
            json!({
                "kind": "Command",
                "value": {"name": "im_locate_message", "payload": {
                    "channel_id": CHANNEL_ID,
                    "message_id": TARGET_ID,
                    "pageSize": 20,
                    "req_id": "g10d-locate-request",
                    "navigationToken": "g10d-navigation"
                }},
                "now_ms": CREATED_AT + 3
            }),
            json!({
                "kind": "Command",
                "value": {"name": "im_send_message", "payload": {
                    "channel_id": CHANNEL_ID,
                    "text": "G-10c appended message",
                    "req_id": "g10c-send-request"
                }},
                "now_ms": CREATED_AT + 6
            }),
            json!({
                "kind": "PortReply",
                "value": {"correlation": refresh.raw(), "outcome": {
                    "variant": "Ok",
                    "rows": "21 durable message rows while away-bottom"
                }},
                "now_ms": CREATED_AT + 11
            }),
        ],
    })
}

/// 构造以目标消息为中心的真实 postContext CommonRes。
fn locate_body() -> TestResult<Bytes> {
    let posts = (5..=15).map(remote_post).collect::<Vec<_>>();
    serde_json::to_vec(&json!({
        "status": "SUCCESS",
        "data": {
            "posts": posts,
            "targetIndex": 5,
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
        "message": format!("G-10d message {index}"),
        "simpleMessage": format!("G-10d message {index}"),
        "viewers": ["all"],
        "mentions": [],
        "eventSeq": index,
        "createAt": CREATED_AT + i64::from(index),
    })
}

/// G-10d 新事实更新 newer 区域，但 anchor identity 保持不变。
#[test]
fn g10d_preserves_anchor_and_exposes_newer_hint() -> TestResult<()> {
    let trace = run_success()?;
    assert_eq!(trace.events.len(), 1);
    let event = &trace.events[0];
    assert_eq!(event["event"], "im:timeline:update");
    assert_eq!(event["data"]["direction"], "newer");
    assert_eq!(event["data"]["anchorPostId"], TARGET_ID);
    assert_eq!(event["data"]["newerCount"], 11);
    assert_eq!(event["data"]["hasNewer"], true);
    Ok(())
}

/// G-10d 事件不携带 ActivateWindow 或其他 renderer 副作用。
#[test]
fn g10d_does_not_emit_activate_window_directive() -> TestResult<()> {
    let trace = run_success()?;
    let encoded = serde_json::to_string(&trace.events).map_err(|error| error.to_string())?;
    for forbidden in [
        "ActivateWindow",
        "InteractionFrame",
        "Surface",
        "mutation",
        "operations",
        "directive",
        "actionCapabilities",
    ] {
        assert!(!encoded.contains(forbidden), "forbidden field {forbidden}");
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-10d receiver 与空 sender canonical artifact。
#[test]
fn g10d_writes_reviewable_dual_persona_artifacts() -> TestResult<()> {
    let trace = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-10d",
        "message_v3_g10d",
        trace.ticks,
        Vec::new(),
        trace.events,
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
