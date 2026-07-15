//! Dialog 指令化投影：在线 post 的可见性、分类、计数 patch 聚焦回归。

mod common;
use common::*;

use helix_core::effect::{GuardedBumpSpec, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};

const AUTH: &str = "auth-user-1";
const OTHER: &str = "other-user-1";

fn post_frame(
    channel_id: u64,
    event_seq: u64,
    sender: &str,
    viewers: serde_json::Value,
    mentions: serde_json::Value,
    expedite_map: serde_json::Value,
    text: &str,
) -> Tick {
    use serde_json::json;
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "channelId": cid(channel_id),
            "event_seq": event_seq,
            "id": format!("msg-{channel_id}-{event_seq}"),
            "temporaryId": format!("tmp-{channel_id}-{event_seq}"),
            "message": text,
            "simpleMessage": text,
            "type": "POST",
            "readBits": "100",
            "userId": sender,
            "userSnapshot": { "userId": sender },
            "viewers": viewers,
            "mentions": mentions,
            "expediteMap": expedite_map,
            "props": { "channel_event_seq": event_seq },
            "createAt": 1_700_000_000_i64 + event_seq as i64,
        },
        "broadcast": {},
        "seq": 1
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

fn emit_payloads(h: &Harness, event_name: &str) -> Vec<serde_json::Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => {
                let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                if value.get("event").and_then(|v| v.as_str()) == Some(event_name) {
                    Some(value)
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect()
}

fn guarded_bumps(h: &Harness) -> Vec<GuardedBumpSpec> {
    h.sink
        .as_slice()
        .iter()
        .flat_map(|effect| match effect {
            Effect::PersistFire { ops } | Effect::Persist { ops, .. } => ops.as_slice(),
            _ => &[][..],
        })
        .filter_map(|op| match op {
            StorageOp::GuardedBump(spec) => Some(spec.clone()),
            _ => None,
        })
        .collect()
}

fn has_message_upsert(h: &Harness) -> bool {
    h.sink.as_slice().iter().any(|effect| match effect {
        Effect::PersistFire { ops } | Effect::Persist { ops, .. } => ops
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message")),
        _ => false,
    })
}

fn dialog_persist_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops
                    .iter()
                    .any(|op| matches!(op, StorageOp::Get(s) if s.table == "channel")) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("DialogPatch 必须注册写后 Get(channel) 的 Persist corr")
}

fn channel_row_reply(
    channel_id: u64,
    unread_count: i64,
    mention_count: i64,
    urgent_count: i64,
    mention_list: &[&str],
    urgent_post_list: &[&str],
    unread_post_id: &str,
    last_root_post_at: i64,
) -> bytes::Bytes {
    let row = serde_json::json!([{
        "id": cid(channel_id),
        "unread_count": unread_count,
        "mention_count": mention_count,
        "urgent_count": urgent_count,
        "mention_list": mention_list,
        "urgent_post_list": urgent_post_list,
        "unread_post_id": unread_post_id,
        "last_root_post_at": last_root_post_at,
    }]);
    bytes::Bytes::from(serde_json::to_vec(&row).unwrap())
}

#[test]
fn visible_post_emits_render_ready_dialog_patch() {
    let mut h = Harness::new_with_identity(&[1], AUTH, "", "", "", "");

    h.step(
        post_frame(
            1,
            1,
            OTHER,
            serde_json::json!(["all"]),
            serde_json::json!([]),
            serde_json::json!({}),
            "hello dialog",
        ),
        1_000,
    );

    assert!(has_message_upsert(&h), "可见 post 必须落 message");
    let corr = dialog_persist_corr(&h);
    assert!(
        emit_payloads(&h, "im:channel:update-by-post").is_empty(),
        "Dialog patch 不能在写前用 delta 直接 emit"
    );

    h.step(
        port_reply_with_body(
            corr,
            channel_row_reply(
                1,
                7,
                2,
                1,
                &["old", "msg-1-1"],
                &["u1"],
                "msg-1-1",
                1_700_000_001,
            ),
        ),
        1_100,
    );
    let patch = emit_payloads(&h, "im:channel:update-by-post");
    assert_eq!(patch.len(), 1, "可见 post 必须发 Dialog patch");
    let data = &patch[0]["data"];
    assert_eq!(data["op"], "dialog.patchByPost");
    assert_eq!(data["source"], "online_post");
    assert_eq!(data["messageClass"], "normal");
    assert_eq!(data["senderUserId"], OTHER);
    assert_eq!(data["lastMessage"], "hello dialog");
    assert_eq!(data["unreadDelta"], 1);
    assert_eq!(data["dialogPatch"]["channelId"], cid(1));
    assert_eq!(data["dialogPatch"]["lastMessage"], "hello dialog");
    assert_eq!(data["dialogPatch"]["unreadCount"], 7);
    assert_eq!(data["dialogPatch"]["mentionCount"], 2);
    assert_eq!(data["dialogPatch"]["urgentCount"], 1);
    assert_eq!(data["dialogPatch"]["mentionList"][1], "msg-1-1");
    assert_eq!(data["lastPost"]["message"], "hello dialog");
}

#[test]
fn own_echo_updates_last_post_without_unread_delta() {
    let mut h = Harness::new_with_identity(&[2], AUTH, "", "", "", "");

    h.step(
        post_frame(
            2,
            1,
            AUTH,
            serde_json::json!(["all"]),
            serde_json::json!([]),
            serde_json::json!({}),
            "from myself",
        ),
        1_000,
    );

    let bumps = guarded_bumps(&h);
    assert_eq!(bumps.len(), 1, "lastPost 仍由同一 GuardedBump 原语落库");
    assert_eq!(bumps[0].bump_delta, 0, "自己的 echo 不增加 unread_count");

    let corr = dialog_persist_corr(&h);
    h.step(
        port_reply_with_body(
            corr,
            channel_row_reply(2, 0, 0, 0, &[], &[], "", 1_700_000_001),
        ),
        1_100,
    );
    let patch = emit_payloads(&h, "im:channel:update-by-post");
    assert_eq!(patch.len(), 1, "自己的 echo 仍要刷新 lastPost");
    let data = &patch[0]["data"];
    assert_eq!(data["senderUserId"], AUTH);
    assert_eq!(data["unreadDelta"], 0);
    assert_eq!(data["dialogPatch"]["unreadCount"], 0);
}

#[test]
fn mentions_and_urgent_posts_classify_and_bump_dialog_counts() {
    let mut mention = Harness::new_with_identity(&[3], AUTH, "", "", "", "");
    mention.step(
        post_frame(
            3,
            1,
            OTHER,
            serde_json::json!(["all"]),
            serde_json::json!([AUTH]),
            serde_json::json!({}),
            "@ you",
        ),
        1_000,
    );
    let mention_bump = guarded_bumps(&mention).remove(0);
    let mention_corr = dialog_persist_corr(&mention);
    mention.step(
        port_reply_with_body(
            mention_corr,
            channel_row_reply(3, 1, 1, 0, &["msg-3-1"], &[], "msg-3-1", 1_700_000_001),
        ),
        1_100,
    );
    let mention_patch = emit_payloads(&mention, "im:channel:update-by-post");
    let mention_data = &mention_patch[0]["data"];
    assert_eq!(mention_data["messageClass"], "mention");
    assert_eq!(mention_data["mentionDelta"], 1);
    assert_eq!(mention_data["dialogPatch"]["mention"], true);
    assert_eq!(mention_data["dialogPatch"]["mentionCount"], 1);
    assert_eq!(mention_data["dialogPatch"]["mentionList"][0], "msg-3-1");
    assert_eq!(mention_data["mentionedUserIds"][0], AUTH);
    assert!(
        mention_bump
            .extra_bumps
            .iter()
            .any(|(col, delta)| *col == "mention_count" && *delta == 1),
        "@ 当前用户时必须在同一 guard 下累加 mention_count"
    );
    assert!(
        mention_bump.set_cols.iter().any(|(col, value)| {
            col == "mention_list"
                && matches!(value, SqlValue::Text(list) if list == r#"["msg-3-1"]"#)
        }),
        "@ 当前用户时必须在同一 guard 下把 mention_list 设为当前消息 singleton"
    );

    let mut urgent = Harness::new_with_identity(&[4], AUTH, "", "", "", "");
    urgent.step(
        post_frame(
            4,
            1,
            OTHER,
            serde_json::json!(["all"]),
            serde_json::json!([]),
            serde_json::json!({ "users": [AUTH] }),
            "urgent for you",
        ),
        1_000,
    );
    let urgent_bump = guarded_bumps(&urgent).remove(0);
    let urgent_corr = dialog_persist_corr(&urgent);
    urgent.step(
        port_reply_with_body(
            urgent_corr,
            channel_row_reply(4, 1, 0, 1, &[], &["msg-4-1"], "msg-4-1", 1_700_000_001),
        ),
        1_100,
    );
    let urgent_patch = emit_payloads(&urgent, "im:channel:update-by-post");
    let urgent_data = &urgent_patch[0]["data"];
    assert_eq!(urgent_data["messageClass"], "urgent");
    assert_eq!(urgent_data["urgentDelta"], 1);
    assert_eq!(urgent_data["dialogPatch"]["urgent"], true);
    assert_eq!(urgent_data["dialogPatch"]["urgentCount"], 1);
    assert_eq!(urgent_data["dialogPatch"]["urgentPostList"][0], "msg-4-1");
    assert_eq!(urgent_data["urgentUserIds"][0], AUTH);
    assert!(
        urgent_bump
            .extra_bumps
            .iter()
            .any(|(col, delta)| *col == "urgent_count" && *delta == 1),
        "加急命中当前用户时必须在同一 guard 下累加 urgent_count"
    );
    assert!(
        urgent_bump.set_cols.iter().any(|(col, value)| {
            col == "urgent_post_list"
                && matches!(value, SqlValue::Text(list) if list == r#"["msg-4-1"]"#)
        }),
        "加急命中当前用户时必须在同一 guard 下把 urgent_post_list 设为当前消息 singleton"
    );
}

#[test]
fn invisible_viewers_only_advance_cursor_without_visible_projection() {
    let mut h = Harness::new_with_identity(&[5], AUTH, "", "", "", "");

    h.step(
        post_frame(
            5,
            1,
            OTHER,
            serde_json::json!(["another-user"]),
            serde_json::json!([AUTH]),
            serde_json::json!({ "users": [AUTH] }),
            "not visible",
        ),
        1_000,
    );

    assert_eq!(h.module.cursor_for(channel_id(5)), Some(1));
    assert!(
        !has_message_upsert(&h),
        "不可见 post 不应落成本地可见 Message"
    );
    assert!(guarded_bumps(&h).is_empty(), "不可见 post 不应 bump Dialog");
    assert!(
        emit_payloads(&h, "im:post:received").is_empty(),
        "不可见 post 不应发消息渲染投影"
    );
    assert!(
        emit_payloads(&h, "im:channel:update-by-post").is_empty(),
        "不可见 post 不应发 Dialog UI patch"
    );
    assert!(
        h.sink.as_slice().iter().any(|effect| {
            matches!(
                effect,
                Effect::PersistFire { ops }
                    if ops.iter().any(|op| matches!(
                        op,
                        StorageOp::MonotonicUpsert(spec)
                            if spec.table == "channel_event_cursor"
                                && spec.value == 1
                    ))
            )
        }),
        "不可见 post 仍要推进 channel_event_cursor 水位"
    );
}
