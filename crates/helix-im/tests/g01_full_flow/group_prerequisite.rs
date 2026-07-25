use helix_core::effect::{Effect, StorageOp};
use serde_json::json;

use crate::support::*;

#[test]
/// 证明 G15a 权威建群事实先于该 channel 的首次 G01 optimistic write。
fn create_group_authority_precedes_first_g01_send() {
    let mut replay = Replay::new();

    replay.step(
        command(
            "im_create_channel",
            json!({
                "type": "O",
                "user_ids": [ACTOR_ID, "g01-member-a"],
                "display_name": "G01 Flow Group",
                "force_create": true
            }),
        ),
        CREATED_AT as u64 - 10,
    );
    let create_corr = replay.only_http_corr("/channel/create");
    let create_body = replay
        .effects()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body.as_ref()).ok()),
            _ => None,
        })
        .expect("create-channel HTTP body");
    assert_eq!(create_body["displayName"], "G01 Flow Group");
    assert_eq!(create_body["userIds"], json!([ACTOR_ID, "g01-member-a"]));

    replay.step(
        ok(
            create_corr,
            http_envelope(json!({
                "status": "SUCCESS",
                "data": {
                    "id": CHANNEL_ID,
                    "type": "P",
                    "displayName": "G01 Flow Group",
                    "teamId": TEAM_ID,
                    "createAt": CREATED_AT - 5
                }
            })),
        ),
        CREATED_AT as u64 - 9,
    );
    let created_tables = replay
        .effects()
        .iter()
        .flat_map(|effect| match effect {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => ops.as_slice(),
            _ => &[],
        })
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(spec) => Some(spec.table),
            StorageOp::BatchUpdate(spec) => Some(spec.table),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        created_tables.contains("channel") && created_tables.contains("channel_member"),
        "authoritative create response must persist channel and membership facts; tables={created_tables:?}"
    );
    let created = replay
        .event("im:channel:created")
        .expect("created projection");
    assert_eq!(created["data"]["channelId"], CHANNEL_ID);

    attach_timeline(&mut replay);
    replay.step(
        command(
            "im_send_message",
            json!({
                "channel_id": CHANNEL_ID,
                "text": TEXT,
                "type": "TEXT",
                "req_id": REQUEST_ID
            }),
        ),
        CREATED_AT as u64 + 10,
    );
    let p1 = replay.only_persist_corr_with(|ops| has_table_write(ops, "message"));
    let persisted_channel = replay.effects().iter().find_map(|effect| match effect {
        Effect::Persist { corr, ops } if *corr == p1 => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" => {
                spec.rows.first().and_then(|row| {
                    row.iter()
                        .find_map(|(column, value)| match (column.as_str(), value) {
                            ("channel_id", helix_core::effect::SqlValue::Text(value)) => {
                                Some(value.as_str())
                            }
                            _ => None,
                        })
                })
            }
            _ => None,
        }),
        _ => None,
    });
    assert_eq!(
        persisted_channel,
        Some(CHANNEL_ID),
        "G01 must use the authoritative channel id returned by G15a"
    );
}
