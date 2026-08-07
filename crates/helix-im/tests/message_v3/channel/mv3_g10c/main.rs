#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const GATE_ID: &str = "MV3-G10c";
const CHANNEL_ID: &str = "chfixx0000000000000000000a";
const NOW_MS: u64 = 1_700_001_800_000;

/// 锁定 G10c 的完整只读参考包，供简介写入场景复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 提取简介写入的 HTTP correlation，确认使用共享 info endpoint。
fn http_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("channel/change/info") => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing info HTTP effect".to_string())
}

/// 提取本地 Persist 屏障，确保服务端完整快照落稳后才发布可见事件。
fn persist_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing info persistence barrier".to_string())
}

/// 解码当前批次中的 MessageV3 事件，供裸 channelUpdate 断言复用。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// G10c 成功必须把 purpose 发给 info endpoint，且完整快照保留 header 与其他字段。
#[test]
fn g10c_success_preserves_header_then_emits_bare_channel_update() -> Result<(), String> {
    let mut harness =
        common::Harness::new_with_identity(&[], "g10c-actor", "g10c-company", "", "", "");
    harness.step(
        common::command_json(
            "im_channel_change_info",
            json!({
                "channel_id": CHANNEL_ID,
                "purpose": "After",
            }),
        ),
        NOW_MS,
    );
    let corr = http_corr(harness.sink.as_slice())?;
    let Effect::Http { req, .. } = &harness.sink.as_slice()[0] else {
        return Err("info command must start with HTTP".to_string());
    };
    let body: Value = serde_json::from_slice(
        req.body
            .as_ref()
            .ok_or_else(|| "info HTTP body missing".to_string())?
            .as_ref(),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        body,
        json!({
            "channelId": CHANNEL_ID,
            "purpose": "After",
        })
    );
    assert!(body.get("header").is_none());
    assert!(body.get("displayName").is_none());

    let response = json!({
        "status": "SUCCESS",
        "data": {
            "id": CHANNEL_ID,
            "type": "P",
            "displayName": "Existing",
            "purpose": "After",
            "header": "Pinned notice",
            "props": {"keep": "yes", "notice": {"text": "Pinned notice"}},
            "status": "OPEN",
            "roles": ["CREATOR"],
            "capabilitiesByRole": {"CREATOR": {"canManageSettings": true}},
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
    assert_eq!(events[0]["data"]["purpose"], "After");
    assert_eq!(events[0]["data"]["header"], "Pinned notice");
    assert_eq!(events[0]["data"]["displayName"], "Existing");
    assert_eq!(events[0]["data"]["props"]["keep"], "yes");
    assert_eq!(events[0]["data"]["settingVersion"], 8);
    assert!(events[0]["data"].get("channel").is_none());
    assert!(events
        .iter()
        .all(|event| event["event"] != "im:channel:settings-updated"));
    Ok(())
}
/// G10c 非法字段在 Helix 边界拒绝 HTTP。
#[test]
fn g10c_invalid_info_intent_has_zero_http_effects() {
    let cases = [
        json!({"purpose": "x"}),
        json!({"channel_id": CHANNEL_ID, "purpose": 1}),
    ];
    for payload in cases {
        let result = helix_im::outbound::handle_outbound(
            "im_channel_change_info",
            serde_json::to_vec(&payload)
                .expect("serialize malformed info intent")
                .as_slice(),
            "http://h/api",
            "http://h",
            None,
            helix_core::Correlation::from_raw(99),
        );
        assert!(
            result.is_err(),
            "invalid payload unexpectedly dispatched: {payload}"
        );
    }
}
