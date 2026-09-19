// 回归锚: Spec 047/T072-T073，G-16c 频道设置更新。

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../../common.rs"]
mod common;

const CHANNEL_ID: &str = "chfixx0000000000000000001c";
type TestResult<T> = Result<T, String>;

/// 返回当前批次唯一 Persist correlation。
fn persist_corr(effects: &[Effect]) -> TestResult<helix_core::Correlation> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing settings persistence".to_string())
}

/// 解码当前批次 MessageV3 events。
fn events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// attach dialog 后回放带版本的 update_channel，并跨越 PersistOk。
fn run_persona(viewer: &str) -> TestResult<(Vec<Value>, u64)> {
    let mut harness =
        common::Harness::new_with_identity(&[0x1c], viewer, "g16c-company", "", "", "");
    harness.step(
        common::command_json("im_query_dialog_list", json!({})),
        1_700_001_620_000,
    );
    let query = persist_corr(harness.sink.as_slice())?;
    let rows = serde_json::to_vec(&vec![json!({
        "id":CHANNEL_ID,
        "display_name":"Before",
        "type":"P",
        "team_id":"g16c-company"
    })])
    .map_err(|error| error.to_string())?;
    harness.step(
        common::port_reply_with_body(query.raw(), Bytes::from(rows)),
        1_700_001_620_001,
    );
    let inbound = serde_json::to_vec(&json!({
        "action":"update_channel",
        "event_seq":61,
        "data":{
            "channel":{
                "id":CHANNEL_ID,
                "displayName":"After",
                "purpose":"G16c purpose",
                "header":"G16c notice",
                "settingVersion":7
            }
        }
    }))
    .map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound))),
        1_700_001_620_002,
    );
    let persist = persist_corr(harness.sink.as_slice())?;
    assert!(!events(harness.sink.as_slice())
        .iter()
        .any(|event| { event["event"] == "im:channel:settings-updated" }));
    harness.step(
        Tick::PortReply {
            corr: persist,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        1_700_001_620_003,
    );
    Ok((events(harness.sink.as_slice()), persist.raw()))
}

/// G-16c sender 与 observer 都从已持久化 authority 收敛到相同设置字段。
#[test]
fn g16c_emits_committed_settings_for_both_personas() -> TestResult<()> {
    for viewer in ["g16c-sender", "g16c-observer"] {
        let (events, _) = run_persona(viewer)?;
        let event = events
            .iter()
            .find(|event| event["event"] == "im:channel:settings-updated")
            .ok_or_else(|| "missing settings event".to_string())?;
        assert_eq!(event["data"]["channelId"], CHANNEL_ID);
        assert_eq!(event["data"]["displayName"], "After");
        assert_eq!(event["data"]["purpose"], "G16c purpose");
        assert_eq!(event["data"]["header"], "G16c notice");
        assert_eq!(event["data"]["settingVersion"], 7);
    }
    Ok(())
}

/// Rust producer 在生成模式下写出 G-16c 双视角 canonical artifact。
#[test]
fn artifact_test() -> TestResult<()> {
    let (sender, corr) = run_persona("g16c-sender")?;
    let (receiver, _) = run_persona("g16c-observer")?;
    let Some(output) = std::env::var_os("HELIX_GATE_ARTIFACT_DIR") else {
        return Ok(());
    };
    let storage = json!([{"table":"channel","columns":["id","display_name","purpose","header","setting_version"]}]);
    let bundle = artifact::gate_bundle(
        "G-16c",
        "message_v3_g16c",
        vec![
            json!({"kind":"Inbound","value":{"action":"update_channel","event_seq":61},"now_ms":1_700_001_620_002_i64}),
            json!({"kind":"PortReply","value":{"correlation":corr,"outcome":{"variant":"Ok"}},"now_ms":1_700_001_620_003_i64}),
        ],
        sender,
        receiver,
        storage.clone(),
        storage.clone(),
        storage,
    );
    artifact::write_bundle(std::path::Path::new(&output), &bundle)
        .map_err(|error| error.to_string())
        .map(|_| ())
}
