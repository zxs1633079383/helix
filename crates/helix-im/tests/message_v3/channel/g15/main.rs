// 回归锚: Spec 047/T066-T067，G-15 创建话题。

use bytes::Bytes;
use helix_core::effect::Effect;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

const PARENT_ID: &str = "chfixx00000000000000000014";
const TOPIC_ID: &str = "chfixx00000000000000000015";
const ROOT_ID: &str = "srvfix00000000000000000015";
type TestResult<T> = Result<T, String>;

/// 取当前批次唯一相关 I/O correlation。
fn correlation(effects: &[Effect]) -> TestResult<helix_core::Correlation> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } | Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing G-15 correlation".to_string())
}

/// 解码当前批次的 MessageV3 事件。
fn events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// 回放 make-topic HTTP、关系持久化和 root durable readback。
fn run_success() -> TestResult<(Vec<Value>, Vec<Value>)> {
    let mut harness =
        common::Harness::new_with_identity(&[], "g15-sender", "g15-company", "", "", "");
    harness.step(
        common::command_json(
            "im_make_topic",
            json!({
                "root_id": ROOT_ID,
                "user_ids": ["g15-receiver"],
                "display_name": "G15 Topic",
                "req_id": "g15-request"
            }),
        ),
        1_700_001_510_000,
    );
    let http = correlation(harness.sink.as_slice())?;
    let body = serde_json::to_vec(&json!({
        "status":"SUCCESS",
        "data":{"rootMessageId":ROOT_ID,"channelId":TOPIC_ID,"memberIds":["g15-sender","g15-receiver"],"participantCount":2,"revision":41}
    }))
    .map_err(|error| error.to_string())?;
    harness.step(
        common::port_reply_with_body(http.raw(), common::enveloped(Bytes::from(body))),
        1_700_001_510_001,
    );
    let persist = correlation(harness.sink.as_slice())?;
    harness.step(common::port_reply_ok(persist.raw()), 1_700_001_510_002);
    let read = correlation(harness.sink.as_slice())?;
    let row = json!([{
        "id":ROOT_ID,
        "channel_id":PARENT_ID,
        "user_id":"g15-sender",
        "type":"TEXT",
        "message":"topic root",
        "topic":serde_json::json!({"channelId":TOPIC_ID,"participantCount":2,"members":["g15-sender","g15-receiver"]}).to_string(),
        "create_at":1_700_001_510_000_i64
    }]);
    let rows = serde_json::to_vec(&row).map_err(|error| error.to_string())?;
    harness.step(
        common::port_reply_with_body(read.raw(), Bytes::from(rows)),
        1_700_001_510_003,
    );
    let sender = events(harness.sink.as_slice());
    Ok((sender.clone(), sender))
}

/// G-15 只复用既有 channelCreated + postUpdated，并在 durable root readback 后保持顺序。
#[test]
fn g15_emits_created_channel_then_updated_root_after_readback() -> TestResult<()> {
    let (events, _) = run_success()?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "im:channel:created");
    assert_eq!(events[0]["data"]["id"], TOPIC_ID);
    assert_eq!(events[0]["data"]["type"], "T");
    assert_eq!(events[0]["data"]["rootId"], PARENT_ID);
    assert_eq!(events[0]["data"]["parentPostId"], ROOT_ID);
    assert_eq!(
        events[0]["data"]["members"],
        json!(["g15-sender", "g15-receiver"])
    );
    assert_eq!(events[0]["data"]["memberCount"], 2);
    assert_eq!(events[0]["data"]["unreadCount"], 0);
    assert_eq!(events[1]["event"], "im:post:updated");
    assert_eq!(events[1]["data"]["id"], ROOT_ID);
    assert_eq!(events[1]["data"]["topic"]["channelId"], TOPIC_ID);
    assert_eq!(events[1]["data"]["topic"]["participantCount"], 2);
    assert!(!events
        .iter()
        .any(|event| event["event"] == "im:channel:topic-created"));
    Ok(())
}

/// Rust producer 在生成模式下写出 G-15 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let (sender, receiver) = run_success()?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let bundle = artifact::gate_bundle(
        "G-15",
        "message_v3_g15",
        vec![
            json!({"kind":"Command","value":{"name":"im_make_topic","payload":{"root_id":ROOT_ID}},"now_ms":1_700_001_510_000_i64}),
            json!({"kind":"PortReply","value":{"outcome":{"variant":"Ok"}},"now_ms":1_700_001_510_003_i64}),
        ],
        sender,
        receiver,
        json!([{"table":"post","columns":["id","channel_id","user_id","message","create_at"]},{"table":"channel","columns":["id","root_id","display_name"]}]),
        json!([{"table":"message","columns":["id","channel_id","user_id","message","create_at"]},{"table":"channel","columns":["id","root_id","display_name"]}]),
        json!([{"table":"message","columns":["id","channel_id","user_id","message","create_at"]},{"table":"channel","columns":["id","root_id","display_name"]}]),
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
