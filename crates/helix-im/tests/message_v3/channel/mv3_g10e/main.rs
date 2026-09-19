//! MV3-G10e：notify-only `update_channel` 的三态持久化与读回收敛。
//!
//! 这里测试 Helix 的真实入站/PortReply 边界，不再把 Phase 3 的人工参考 JSON 完整性
//! 当作行为测试。Native target 负责把同一合同继续接到 HTTP/Go/WS/SQLite。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::Tick;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

const CHANNEL_ID: &str = "chfixx0000000000000000000e";
const ACTOR_ID: &str = "g10e-actor";
const COMPANY_ID: &str = "g10e-company";
const NOW_MS: u64 = 1_700_001_800_000;

/// 构造 Go `UpdateChannelNotifyVo` 的真实 notify-only 入站帧。
fn notify_frame(user_id: &str, notify: Value, extra: Value) -> Tick {
    let mut data = serde_json::Map::new();
    data.insert("id".to_string(), json!(CHANNEL_ID));
    data.insert("userId".to_string(), json!(user_id));
    data.insert("notify".to_string(), notify);
    if let Value::Object(fields) = extra {
        data.extend(fields);
    }
    Tick::Inbound(helix_core::tick::InboundBytes(
        serde_json::to_vec(&json!({"action": "update_channel", "data": data}))
            .expect("serialize G10e notify frame")
            .into(),
    ))
}

/// 返回本批次唯一 notify Persist correlation。
fn persist_corr(effects: &[Effect]) -> Result<helix_core::Correlation, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(*corr),
            _ => None,
        })
        .ok_or_else(|| "missing notify persistence barrier".to_string())
}

/// 解码本批次所有 MessageV3 domain events。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// 从 notify Persist 中取出 channel partial patch，验证只写通知列。
fn channel_patch(effects: &[Effect]) -> Result<&helix_core::effect::BatchUpdateSpec, String> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(spec),
                _ => None,
            }),
            _ => None,
        })
        .ok_or_else(|| "missing channel notify partial patch".to_string())
}

/// 构造完整 channel readback，确保事件不能从稀疏 WS payload 猜未携带字段。
fn channel_readback(notify: &str) -> Bytes {
    serde_json::to_vec(&json!([{
        "id": CHANNEL_ID,
        "team_id": COMPANY_ID,
        "user_id": ACTOR_ID,
        "type": "P",
        "notify_props": notify,
        "unread_count": 7,
        "is_top": 1,
        "last_post_at": 42,
    }]))
    .expect("serialize channel notify readback")
    .into()
}

/// G10e 三态都只写 notify_props/member.notify，PersistOk 前不发布事件，成功后按 SQLite 读回发布。
#[test]
fn g10e_notify_states_persist_after_barrier_and_emit_scalar_contract() -> Result<(), String> {
    for notify in ["NORMAL", "STRONG", "IGNORE"] {
        let mut harness =
            common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
        harness.step(notify_frame(ACTOR_ID, json!(notify), json!({})), NOW_MS);
        let patch = channel_patch(harness.sink.as_slice())?;
        assert_eq!(
            patch.patch.len(),
            1,
            "notify must not alter unrelated channel columns"
        );
        assert!(matches!(
            patch.patch.first(),
            Some((column, SqlValue::Text(value))) if column == "notify_props" && value == notify
        ));
        let member_row = harness
            .sink
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpsert(spec) if spec.table == "channel_member" => {
                        spec.rows.first()
                    }
                    _ => None,
                }),
                _ => None,
            })
            .ok_or_else(|| "missing channel_member notify mirror".to_string())?;
        assert!(member_row.iter().any(|(column, value)| {
            column == "notify" && matches!(value, SqlValue::Text(value) if value == notify)
        }));
        assert!(member_row
            .iter()
            .all(|(column, _)| { !matches!(column.as_str(), "unread_count" | "channel_is_top") }));
        assert!(emitted(harness.sink.as_slice()).is_empty());

        let corr = persist_corr(harness.sink.as_slice())?;
        harness.step(
            common::port_reply_with_body(corr.raw(), channel_readback(notify)),
            NOW_MS + 1,
        );
        let events = emitted(harness.sink.as_slice());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "im:channel:update");
        assert_eq!(events[0]["data"]["notifyProps"], notify);
        assert_eq!(events[0]["data"]["unreadCount"], 7);
        assert_eq!(events[0]["data"]["channelIsTop"], true);
    }
    Ok(())
}

/// Persist 失败或无 readback 时必须 fail-closed，不泄漏实时 channel update。
#[test]
fn g10e_notify_failure_does_not_emit() -> Result<(), String> {
    let mut harness = common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
    harness.step(notify_frame(ACTOR_ID, json!("IGNORE"), json!({})), NOW_MS);
    let corr = persist_corr(harness.sink.as_slice())?;
    harness.step(common::port_reply_err(corr.raw()), NOW_MS + 1);
    assert!(emitted(harness.sink.as_slice()).is_empty());

    let mut no_readback =
        common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
    no_readback.step(notify_frame(ACTOR_ID, json!("IGNORE"), json!({})), NOW_MS);
    let corr = persist_corr(no_readback.sink.as_slice())?;
    no_readback.step(
        common::port_reply_with_body(corr.raw(), Bytes::from_static(b"[]")),
        NOW_MS + 1,
    );
    assert!(emitted(no_readback.sink.as_slice()).is_empty());
    Ok(())
}

/// 非法枚举、错误 actor、混合字段和缺失 notify 都是零写零发；不会把坏帧降级成普通 PATCH。
#[test]
fn g10e_invalid_notify_is_fail_closed() {
    let cases = [
        notify_frame(ACTOR_ID, json!("invalid"), json!({})),
        notify_frame(ACTOR_ID, json!(null), json!({})),
        notify_frame("other-user", json!("IGNORE"), json!({})),
        notify_frame(ACTOR_ID, json!("IGNORE"), json!({"displayName": "mixed"})),
        notify_frame(ACTOR_ID, json!("IGNORE"), json!({"channel": "mixed"})),
    ];
    for (index, frame) in cases.into_iter().enumerate() {
        let mut harness =
            common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
        use helix_core::Module;
        harness.sink.clear();
        let result = harness.module.handle(&frame, NOW_MS, &mut harness.sink);
        if index < 2 {
            assert!(result.is_err(), "malformed notify must be rejected");
        } else {
            assert!(
                result.is_ok(),
                "scoped/mixed notify should fail closed without panic"
            );
        }
        assert!(harness.sink.as_slice().is_empty());
    }

    let mut missing = common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
    missing.step(
        Tick::Inbound(helix_core::tick::InboundBytes(
            serde_json::to_vec(&json!({
                "action": "update_channel",
                "data": {"id": CHANNEL_ID, "userId": ACTOR_ID}
            }))
            .expect("serialize missing notify")
            .into(),
        )),
        NOW_MS,
    );
    assert!(missing.sink.as_slice().is_empty());
}

/// 重新进入列表只消费 durable channel row，正式输出保持与 increment/PC 一致的 scalar notifyProps。
#[test]
fn g10e_reentry_query_reads_scalar_notify_props() -> Result<(), String> {
    let mut harness = common::Harness::new_with_identity(&[0x0e], ACTOR_ID, COMPANY_ID, "", "", "");
    harness.step(
        common::command_json("im_query_dialog_list", json!({"req_id": "g10e-reentry"})),
        NOW_MS,
    );
    let corr = persist_corr(harness.sink.as_slice())?;
    harness.step(
        common::port_reply_with_body(corr.raw(), channel_readback("STRONG")),
        NOW_MS + 1,
    );
    let result = emitted(harness.sink.as_slice())
        .into_iter()
        .find(|event| event["event"] == "im:read:result")
        .ok_or_else(|| "missing reentry read result".to_string())?;
    assert_eq!(result["data"]["req_id"], "g10e-reentry");
    assert_eq!(result["data"]["body"]["items"][0]["notifyProps"], "STRONG");
    Ok(())
}
