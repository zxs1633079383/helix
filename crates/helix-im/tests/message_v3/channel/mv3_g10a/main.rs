#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const GATE_ID: &str = "MV3-G10a";
const CHANNEL_ID: &str = "chfixx0000000000000000000a";
const NOW_MS: u64 = 1_700_001_800_000;

/// 锁定 G10a 的完整只读参考包，供后续群名写入测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 从当前批次提取唯一 HTTP correlation，确保写入沿现有命令回路回灌。
fn http_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("channel/change/displayName") => {
                Some(*corr)
            }
            _ => None,
        })
        .ok_or_else(|| "missing display-name HTTP effect".to_string())
}

/// 从当前批次提取唯一本地设置持久化屏障，禁止 HTTP 回包直接发布可见投影。
fn persist_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing display-name persistence barrier".to_string())
}

/// 解码本批次的 MessageV3 事件，供裸 DialogItem 与旧事件缺失断言复用。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// G10a 成功只在 Go authority 与本地 PersistOk 后发既有 channelUpdate，且 payload 是裸 DialogItem。
#[test]
fn g10a_success_persists_then_emits_bare_channel_update() -> Result<(), String> {
    let mut harness =
        common::Harness::new_with_identity(&[], "g10a-actor", "g10a-company", "", "", "");
    harness.step(
        common::command_json(
            "im_channel_change_display_name",
            json!({
                "channel_id": CHANNEL_ID,
                "display_name": "After",
            }),
        ),
        NOW_MS,
    );
    let corr = http_corr(harness.sink.as_slice())?;
    let Effect::Http { req, .. } = &harness.sink.as_slice()[0] else {
        return Err("display-name command must start with HTTP".to_string());
    };
    let body: Value = serde_json::from_slice(
        req.body
            .as_ref()
            .ok_or_else(|| "display-name HTTP body missing".to_string())?
            .as_ref(),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(body, json!({"id": CHANNEL_ID, "displayName": "After"}));

    let response = json!({
        "status": "SUCCESS",
        "data": {
            "id": CHANNEL_ID,
            "type": "P",
            "displayName": "After",
            "purpose": "",
            "header": "",
            "settingVersion": 8,
            "canManageSettings": true,
            "canManageMembers": true,
        }
    });
    harness.step(
        common::port_reply_with_body(
            corr.raw(),
            common::enveloped(Bytes::from(
                serde_json::to_vec(&response).map_err(|error| error.to_string())?,
            )),
        ),
        NOW_MS + 1,
    );
    let persist = persist_corr(harness.sink.as_slice())?;
    assert!(emitted(harness.sink.as_slice()).is_empty());

    harness.step(
        Tick::PortReply {
            corr: persist,
            outcome: PortOutcome::Ok(ReplyBytes::default()),
        },
        NOW_MS + 2,
    );
    let events = emitted(harness.sink.as_slice());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:channel:update");
    assert_eq!(events[0]["data"]["id"], CHANNEL_ID);
    assert_eq!(events[0]["data"]["displayName"], "After");
    assert_eq!(events[0]["data"]["settingVersion"], 8);
    assert!(events[0]["data"].get("channel").is_none());
    assert!(events
        .iter()
        .all(|event| event["event"] != "im:channel:settings-updated"));
    Ok(())
}
