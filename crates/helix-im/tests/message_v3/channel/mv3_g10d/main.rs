#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

#[path = "../../../common.rs"]
mod common;

use helix_core::effect::Row;
use helix_core::Effect;
use helix_core::Tick;
use serde_json::{json, Value};

const GATE_ID: &str = "MV3-G10d";
const CHANNEL_ID: &str = "chfixx0000000000000000000d";
const ACTOR_ID: &str = "g10d-actor";
const NOW_MS: u64 = 1_700_001_800_000;

/// 锁定 G10d 的完整只读参考包，供后续置顶偏好测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 解码当前 batch 的 MessageV3 事件，确认 top 成功只发裸 channel update。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// 从当前 batch 取唯一 update_channel Persist correlation。
fn persist_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } | Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing channel-member top persistence".to_string())
}

/// 构造 Go `update_channel` 的 actor-scoped top authority frame。
fn top_frame(user_id: Option<&str>, top: Value, extra: Value) -> Tick {
    let mut channel = serde_json::Map::new();
    channel.insert("id".to_string(), json!(CHANNEL_ID));
    channel.insert("channelIsTop".to_string(), top);
    if let Some(user_id) = user_id {
        channel.insert("userId".to_string(), json!(user_id));
    }
    if let Value::Object(fields) = extra {
        channel.extend(fields);
    }
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({
            "action": "update_channel",
            "data": Value::Object(channel)
        }))
        .expect("serialize G10d update_channel")
        .into(),
    ))
}

/// 抽取 channel_member upsert 行，供 true/false 与共享 channel 防污染断言复用。
fn member_row(effects: &[Effect]) -> Result<&Row, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistAtomic { ops, .. } => {
                ops.iter().find_map(|op| match op {
                    helix_core::effect::StorageOp::BatchUpsert(spec)
                        if spec.table == "channel_member" =>
                    {
                        spec.rows.first()
                    }
                    _ => None,
                })
            }
            _ => None,
        })
        .ok_or_else(|| "missing channel_member upsert row".to_string())
}

/// G10d true/false 都只写 actor 的 channel_member.channel_is_top，不触碰共享 channel.is_top。
#[test]
fn g10d_top_preference_persists_actor_scope_and_emits_once() -> Result<(), String> {
    for top in [true, false] {
        let mut harness =
            common::Harness::new_with_identity(&[0x0d], ACTOR_ID, "g10d-company", "", "", "");
        harness.step(top_frame(Some(ACTOR_ID), json!(top), json!({})), NOW_MS);
        let persist_ops = harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { ops, .. } | Effect::PersistAtomic { ops, .. } => Some(ops),
                _ => None,
            })
            .ok_or_else(|| "missing top persistence ops".to_string())?;
        assert_eq!(persist_ops.len(), 1);
        assert!(matches!(
            persist_ops.first(),
            Some(helix_core::effect::StorageOp::BatchUpsert(spec))
                if spec.table == "channel_member"
        ));
        let row = member_row(harness.sink.as_slice())?;
        let mut row_columns = row
            .iter()
            .map(|(column, _)| column.as_str())
            .collect::<Vec<_>>();
        row_columns.sort_unstable();
        assert_eq!(
            row_columns,
            ["channel_id", "channel_is_top", "updated_at", "user_id"]
        );
        assert!(row.iter().all(|(column, _)| column != "is_top"));
        assert!(row.iter().any(|(column, value)| {
            column == "channel_id"
                && matches!(value, helix_core::effect::SqlValue::Text(value) if value == CHANNEL_ID)
        }));
        assert!(row.iter().any(|(column, value)| {
            column == "user_id"
                && matches!(value, helix_core::effect::SqlValue::Text(value) if value == ACTOR_ID)
        }));
        assert!(row.iter().any(|(column, value)| {
            column == "channel_is_top"
                && matches!(value, helix_core::effect::SqlValue::Integer(value) if *value == top as i64)
        }));
        assert!(row.iter().any(|(column, value)| {
            column == "updated_at"
                && matches!(value, helix_core::effect::SqlValue::Integer(value) if *value == NOW_MS as i64)
        }));
        assert!(emitted(harness.sink.as_slice()).is_empty());

        let corr = persist_corr(harness.sink.as_slice())?;
        harness.step(common::port_reply_ok(corr.raw()), NOW_MS + 1);
        let events = emitted(harness.sink.as_slice());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "im:channel:update");
        assert_eq!(events[0]["data"]["channelId"], CHANNEL_ID);
        assert_eq!(events[0]["data"]["userId"], ACTOR_ID);
        assert_eq!(events[0]["data"]["channelIsTop"], top);
        assert!(events[0]["data"].get("channel").is_none());
        assert!(!events
            .iter()
            .any(|event| event["event"] == "im:read:channel"));
    }
    Ok(())
}

/// 缺 actor、他人 actor、非法 top 或 shared 字段混入时必须 zero Persist/zero emit。
#[test]
fn g10d_invalid_or_mixed_top_intent_has_zero_effects() {
    let cases = [
        top_frame(None, json!(true), json!({})),
        top_frame(Some("g10d-other"), json!(true), json!({})),
        top_frame(Some(ACTOR_ID), json!("true"), json!({})),
        top_frame(Some(ACTOR_ID), json!(true), json!({"displayName": "mixed"})),
    ];
    for frame in cases {
        let mut harness =
            common::Harness::new_with_identity(&[0x0d], ACTOR_ID, "g10d-company", "", "", "");
        harness.step(frame, NOW_MS);
        assert!(harness.sink.as_slice().is_empty());
    }
}

/// 完整共享频道更新可携带 viewer 的 channelIsTop，但不得被误判成缺 actor 的置顶补丁。
#[test]
fn g10d_full_shared_update_with_top_reaches_channel_projection() -> Result<(), String> {
    let mut harness =
        common::Harness::new_with_identity(&[0x0d], ACTOR_ID, "g10d-company", "", "", "");
    harness.step(
        top_frame(
            None,
            json!(false),
            json!({
                "displayName": "共享频道新名称",
                "purpose": "共享频道新简介",
                "orient": "共享频道新导向",
                "owner": {"id": ACTOR_ID, "teamId": "g10d-company"},
                "settingVersion": 2,
                "updateAt": NOW_MS
            }),
        ),
        NOW_MS,
    );

    let corr = persist_corr(harness.sink.as_slice())?;
    harness.step(common::port_reply_ok(corr.raw()), NOW_MS + 1);
    assert!(emitted(harness.sink.as_slice()).is_empty());
    let read = persist_corr(harness.sink.as_slice())?;
    harness.step(common::port_reply_with_body(read.raw(),serde_json::to_vec(&json!([{
        "id":CHANNEL_ID,"snapshot_updated_at":NOW_MS,"display_name":"共享频道新名称","purpose":"共享频道新简介","orient":"共享频道新导向"
    }])).unwrap().into()),NOW_MS+2);

    let events = emitted(harness.sink.as_slice());
    let channel_updates = events
        .iter()
        .filter(|event| event["event"] == "im:channel:update")
        .collect::<Vec<_>>();
    assert_eq!(
        channel_updates.len(),
        1,
        "共享更新只应发一个 canonical channel 事件"
    );
    let channel = channel_updates
        .first()
        .copied()
        .ok_or_else(|| "missing shared im:channel:update".to_string())?;
    assert_eq!(channel["data"]["channelId"], CHANNEL_ID);
    assert_eq!(channel["data"]["displayName"], "共享频道新名称");
    assert_eq!(channel["data"]["purpose"], "共享频道新简介");
    assert_eq!(channel["data"]["orient"], "共享频道新导向");
    assert_eq!(channel["data"]["owner"]["userId"], ACTOR_ID);
    assert_eq!(channel["data"]["channelIsTop"], false);
    assert!(events
        .iter()
        .any(|event| event["event"] == "im:channel:settings-updated"));
    Ok(())
}
