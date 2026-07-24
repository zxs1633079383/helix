//! 回归锚: G-15a —— 建群 HTTP 已成功但 WebSocket 断线时，仍按当前 channel 回灌本地投影。

mod common;
use common::*;

use helix_core::effect::StorageOp;
use helix_core::{Effect, EffectSink};

fn storage_ops(sink: &EffectSink) -> Vec<StorageOp> {
    sink.as_slice()
        .iter()
        .flat_map(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => ops.clone(),
            _ => Vec::new(),
        })
        .collect()
}

fn has_write_to_table(ops: &[StorageOp], table: &str) -> bool {
    ops.iter().any(|op| match op {
        StorageOp::BatchUpdate(spec) => spec.table == table,
        StorageOp::BatchUpsert(spec) => spec.table == table,
        _ => false,
    })
}

fn emitted_data(sink: &EffectSink, event_name: &str) -> Option<serde_json::Value> {
    sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Emit { event } => {
            let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            (envelope.get("event").and_then(serde_json::Value::as_str) == Some(event_name))
                .then(|| envelope.get("data").cloned().unwrap_or_default())
        }
        _ => None,
    })
}

#[test]
fn create_http_success_persists_only_created_channel_and_emits_projection() {
    let mut harness = Harness::new_with_identity(&[], "creator", "team-1", "", "", "");
    let request_effects = harness.step(
        command_json(
            "im_create_channel",
            serde_json::json!({
                "type": "O",
                "user_ids": ["creator", "member-a", "member-b"],
                "display_name": "断线也可见",
                "force_create": true
            }),
        ),
        1_700_000_000_000,
    );
    assert_eq!(request_effects, vec![EffectKind::Http(1)]);

    let response = serde_json::to_vec(&serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "id": cid(71),
            "type": "P",
            "displayName": "断线也可见",
            "teamId": "team-1",
            "createAt": 1_700_000_000_000_i64
        }
    }))
    .unwrap();
    let response_effects = harness.step(
        port_reply_with_body(1, enveloped(bytes::Bytes::from(response))),
        1_700_000_000_100,
    );

    assert!(
        response_effects
            .iter()
            .all(|effect| !matches!(effect, EffectKind::Http(_) | EffectKind::Request(_))),
        "建群成功回包只能落当前 channel，不能触发全量 dialog 请求: {response_effects:?}"
    );
    let ops = storage_ops(&harness.sink);
    assert!(has_write_to_table(&ops, "channel"));
    assert!(has_write_to_table(&ops, "channel_member"));

    let projection = emitted_data(&harness.sink, "im:channel:created")
        .expect("建群 HTTP 成功必须产生 render-ready projection");
    assert_eq!(projection["channelId"], cid(71));
    assert_eq!(projection["displayName"], "断线也可见");
    assert_eq!(projection["memberCount"], 3);
    assert_eq!(
        projection["channel"]["members"].as_array().unwrap().len(),
        3
    );
}

#[test]
fn create_http_business_failure_does_not_fabricate_channel() {
    let mut harness = Harness::new(&[]);
    let _ = harness.step(
        command_json(
            "im_create_channel",
            serde_json::json!({
                "type": "O",
                "user_ids": ["creator"],
                "display_name": "失败群",
                "force_create": true
            }),
        ),
        1_700_000_000_000,
    );
    let failed = serde_json::to_vec(&serde_json::json!({
        "status": "failed",
        "message": "create rejected"
    }))
    .unwrap();

    let effects = harness.step(
        port_reply_with_body(1, enveloped(bytes::Bytes::from(failed))),
        1_700_000_000_100,
    );

    assert!(effects.is_empty());
    assert!(emitted_data(&harness.sink, "im:channel:created").is_none());
}

#[test]
fn create_http_counts_response_owner_when_request_contains_only_invitees() {
    let mut harness = Harness::new_with_identity(&[], "creator", "team-1", "", "", "");
    let _ = harness.step(
        command_json(
            "im_create_channel",
            serde_json::json!({
                "type": "O",
                "user_ids": ["member-a"],
                "display_name": "邀请成员",
                "force_create": true
            }),
        ),
        1_700_000_000_000,
    );
    let response = serde_json::to_vec(&serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "id": cid(72),
            "type": "P",
            "displayName": "邀请成员",
            "teamId": "team-1",
            "owner": { "id": "creator", "role": "OWNER" }
        }
    }))
    .unwrap();

    let _ = harness.step(
        port_reply_with_body(1, enveloped(bytes::Bytes::from(response))),
        1_700_000_000_100,
    );

    let projection =
        emitted_data(&harness.sink, "im:channel:created").expect("建群回包必须发布成员总数");
    assert_eq!(projection["memberCount"], 2);
    // `members` 保留请求邀请者原始形状；owner 仍从回包独立落库。
    assert_eq!(
        projection["channel"]["members"].as_array().unwrap().len(),
        1
    );
}
