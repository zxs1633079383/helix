#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use bytes::Bytes;
use helix_core::effect::Effect;
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const GATE_ID: &str = "MV3-G10b";
const CHANNEL_ID: &str = "chfixx0000000000000000000a";
const NOW_MS: u64 = 1_700_001_800_000;

/// 锁定 G10b 的完整只读参考包，供后续 notice CAS 测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 从当前批次提取公告 HTTP correlation，确保成功与冲突共用现役 command 回路。
fn http_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("channel/change/notice") => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing notice HTTP effect".to_string())
}

/// 从当前批次提取本地 Persist 屏障，禁止 HTTP 回包直接发布可见频道投影。
fn persist_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing notice persistence barrier".to_string())
}

/// 解码本批次的裸 channelUpdate 事件，供正向投影断言复用。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// G10b 成功必须把 nested notice/text 与 expectedVersion 发给 Go，并在 PersistOk 后发布完整裸 DialogItem。
#[test]
fn g10b_cas_success_preserves_props_then_emits_bare_channel_update() -> Result<(), String> {
    let mut harness =
        common::Harness::new_with_identity(&[], "g10b-actor", "g10b-company", "", "", "");
    harness.step(
        common::command_json(
            "im_channel_change_notice",
            json!({
                "channel_id": CHANNEL_ID,
                "notice": {"text": "After"},
                "expected_version": 7,
            }),
        ),
        NOW_MS,
    );
    let corr = http_corr(harness.sink.as_slice())?;
    let Effect::Http { req, .. } = &harness.sink.as_slice()[0] else {
        return Err("notice command must start with HTTP".to_string());
    };
    let body: Value = serde_json::from_slice(
        req.body
            .as_ref()
            .ok_or_else(|| "notice HTTP body missing".to_string())?
            .as_ref(),
    )
    .map_err(|error| error.to_string())?;
    assert_eq!(
        body,
        json!({
            "id": CHANNEL_ID,
            "notice": {"text": "After"},
            "expectedVersion": 7,
        })
    );

    let response = json!({
        "status": "SUCCESS",
        "data": {
            "id": CHANNEL_ID,
            "type": "P",
            "displayName": "Existing",
            "purpose": "",
            "header": "",
            "props": {"keep": "yes", "notice": {"text": "After"}},
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
    assert_eq!(events[0]["data"]["settingVersion"], 8);
    assert_eq!(events[0]["data"]["props"]["keep"], "yes");
    assert_eq!(events[0]["data"]["props"]["notice"]["text"], "After");
    assert!(events[0]["data"].get("channel").is_none());
    assert!(events
        .iter()
        .all(|event| event["event"] != "im:channel:settings-updated"));
    Ok(())
}

/// G10b 陈旧 CAS 回包必须零 Persist、零 event，不得由旧 update_channel_notice 路由伪造成功。
#[test]
fn g10b_stale_cas_conflict_has_zero_local_effects() -> Result<(), String> {
    let mut harness =
        common::Harness::new_with_identity(&[], "g10b-actor", "g10b-company", "", "", "");
    harness.step(
        common::command_json(
            "im_channel_change_notice",
            json!({
                "channel_id": CHANNEL_ID,
                "notice": {"text": "Stale"},
                "expected_version": 7,
            }),
        ),
        NOW_MS,
    );
    let corr = http_corr(harness.sink.as_slice())?;
    let conflict = json!({
        "status": "FAILURE",
        "data": {
            "code": "CHANNEL_SETTINGS_VERSION_CONFLICT",
            "expectedVersion": 7,
            "currentVersion": 8,
        }
    });
    harness.step(
        common::port_reply_with_body(
            corr.raw(),
            common::enveloped(Bytes::from(
                serde_json::to_vec(&conflict).map_err(|error| error.to_string())?,
            )),
        ),
        NOW_MS + 1,
    );
    assert!(harness.sink.as_slice().is_empty());
    Ok(())
}

/// G10b 入站缺失/错误 notice 或 expected_version 时在 Helix 边界拒绝 HTTP。
#[test]
fn g10b_invalid_notice_intent_has_zero_http_effects() {
    let cases = [
        json!({"channel_id": CHANNEL_ID, "expected_version": 7}),
        json!({"channel_id": CHANNEL_ID, "notice": {"message": "wrong"}, "expected_version": 7}),
        json!({"channel_id": CHANNEL_ID, "notice": {"text": "x"}}),
        json!({"channel_id": CHANNEL_ID, "notice": {"text": "x"}, "expected_version": -1}),
    ];
    for payload in cases {
        let result = helix_im::outbound::handle_outbound(
            "im_channel_change_notice",
            serde_json::to_vec(&payload)
                .expect("serialize malformed notice intent")
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
