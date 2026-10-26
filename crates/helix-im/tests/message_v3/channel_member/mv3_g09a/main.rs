#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use bytes::Bytes;
use common::{port_reply_ok, port_reply_with_body, Harness};
use helix_core::effect::{SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::{json, Value};

const GATE_ID: &str = "MV3-G09a";

/// 锁定 G09a 的完整只读参考包，供后续成员变更测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}

/// 驱动成员变更到最终 roster 读回，返回 canonical renderer 事件。
fn run_member_update() -> Value {
    let channel_id = common::cid(9);
    let mut harness = Harness::new_with_identity(&[9], "owner", "team-a", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "v": 1,
                "action": "channel_member_update",
                "data": {
                    "id": channel_id,
                    "teamId": "team-a",
                    "event_seq": 91,
                    "members": [{"id":"member-a","teamId":"team-a","role":"MEMBER"}],
                    "owner": {"id":"owner","teamId":"team-a","role":"OWNER"},
                    "memberChange": {"leave":[{"id":"removed-user"}]}
                }
            }))
            .unwrap(),
        ))),
        1_000,
    );
    let persist_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("member update must persist atomically");
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    harness.step(port_reply_ok(persist_corr), 1_001);
    let channel_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("PersistOk must start channel readback");
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));
    harness.step(
        port_reply_with_body(
            channel_corr,
            Bytes::from(
                serde_json::to_vec(&[json!({"id":channel_id,"display_name":"G09a"})]).unwrap(),
            ),
        ),
        1_002,
    );
    let roster_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| {
                    matches!(op,
            helix_core::effect::StorageOp::Scan(spec) if spec.table == "channel_member")
                }) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("channel readback must start durable roster scan");
    harness.step(port_reply_with_body(
        roster_corr,
        Bytes::from(serde_json::to_vec(&[
            json!({"channel_id":channel_id,"user_id":"owner","team_id":"team-a","role":"OWNER","nick_name":"群主"}),
            json!({"channel_id":channel_id,"user_id":"member-a","team_id":"team-a","role":"MEMBER","nick_name":"成员A"}),
            json!({"channel_id":channel_id,"user_id":"admin-a","team_id":"team-a","role":"ADMIN","nick_name":"管理员A"}),
        ]).unwrap()),
    ), 1_003);
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .expect("roster scan must emit canonical event")
}

/// G09a 只允许 SQLite 读回后的绝对 roster 成为 renderer authority。
#[test]
fn member_update_emits_absolute_durable_roster_after_readback() {
    let event = run_member_update();
    assert_eq!(event["event"], "im:channel:member-updated");
    assert_eq!(event["data"]["channel"]["memberCount"], 3);
    assert_eq!(event["data"]["channel"]["owner"]["userId"], "owner");
    assert_eq!(
        event["data"]["channel"]["adminUsers"][0]["userId"],
        "admin-a"
    );
    assert_eq!(event["data"]["channel"]["members"][0]["userId"], "member-a");
    assert!(event["data"]["channel"]
        .to_string()
        .find("removed-user")
        .is_none());
    assert_eq!(event["data"]["projectionSource"], "channel_member_update");
}

/// 被管理者移出复用 channel_close(left)，并在同一事务清理 viewer 自己的成员投影。
#[test]
fn kicked_viewer_persists_left_tombstone_before_terminal_emit() {
    let channel_id = common::cid(19);
    let mut harness = Harness::new_with_identity(&[19], "removed-user", "team-a", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "v": 1,
                "action": "channel_close",
                "data": {
                    "channelId": channel_id,
                    "deleteAt": 1_700_001_609_000_i64,
                    "terminalReason": "left"
                }
            }))
            .unwrap(),
        ))),
        2_000,
    );
    let (corr, patch, member_delete) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => {
                let patch = ops.iter().find_map(|op| match op {
                    StorageOp::BatchUpdate(spec) if spec.table == "channel" => {
                        Some(spec.patch.clone())
                    }
                    _ => None,
                })?;
                let member_delete = ops.iter().find_map(|op| match op {
                    StorageOp::BatchDelete(spec) if spec.table == "channel_member" => {
                        Some(spec.clone())
                    }
                    _ => None,
                })?;
                Some((corr.raw(), patch, member_delete))
            }
            _ => None,
        })
        .expect("targeted left must atomically persist tombstone and delete viewer membership");
    assert_eq!(patch.len(), 1);
    assert!(matches!(
        patch.first(),
        Some((column, SqlValue::Integer(value))) if column == "is_remove" && *value == 1
    ));
    assert!(patch
        .iter()
        .all(|(column, _)| column != "delete_at" && column != "is_active"));
    assert_eq!(member_delete.scope_col, "channel_id");
    assert!(matches!(
        &member_delete.scope_val,
        SqlValue::Text(value) if value == &channel_id.to_string()
    ));
    assert_eq!(member_delete.key_col, "user_id");
    assert_eq!(member_delete.key_vals.len(), 1);
    assert!(matches!(
        member_delete.key_vals.first(),
        Some(SqlValue::Text(value)) if value == "removed-user"
    ));
    assert!(!harness
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));

    harness.step(port_reply_ok(corr), 2_001);
    let event: Value = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .expect("PersistOk must emit the canonical left terminal event");
    assert_eq!(event["event"], "im:channel:closed");
    assert_eq!(event["data"]["channelId"], channel_id);
    assert_eq!(event["data"]["terminalReason"], "left");
    assert!(event["data"].get("closed").is_none());
}

/// 被重新加入的 viewer 必须在持久成功后同时恢复 dialog 可见性与内存增量聚合。
#[test]
fn rejoined_viewer_clears_local_left_state_after_atomic_persist() {
    let channel_id = common::channel_id(29);
    let mut harness = Harness::new_with_identity(&[29], "rejoined-user", "team-a", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "v": 1,
                "action": "channel_close",
                "data": {"channelId": channel_id.as_str(), "terminalReason": "left"}
            }))
            .unwrap(),
        ))),
        3_000,
    );
    let left_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("left must persist before rejoin");
    harness.step(port_reply_ok(left_corr), 3_001);
    assert!(harness.module.terminal_event_seq_for(channel_id).is_some());

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&json!({
                "v": 1,
                "action": "channel_member_update",
                "data": {
                    "id": channel_id.as_str(),
                    "teamId": "team-a",
                    "event_seq": 92,
                    "memberChange": {
                        "join": [{"id":"rejoined-user","teamId":"team-a","role":"MEMBER"}]
                    }
                }
            }))
            .unwrap(),
        ))),
        3_002,
    );
    let (rejoin_corr, rejoin_patch) = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpdate(spec) if spec.table == "channel"
                    && spec.patch.iter().any(|(column, _)| column == "is_remove") => {
                    Some((corr.raw(), spec.patch.clone()))
                }
                _ => None,
            }),
            _ => None,
        })
        .expect("self rejoin must atomically reset the viewer-local channel projection");
    assert!(rejoin_patch
        .iter()
        .any(|(column, value)| { column == "is_remove" && matches!(value, SqlValue::Integer(0)) }));
    assert!(harness.module.terminal_event_seq_for(channel_id).is_some());

    harness.step(port_reply_ok(rejoin_corr), 3_003);
    assert_eq!(harness.module.terminal_event_seq_for(channel_id), None);
}
