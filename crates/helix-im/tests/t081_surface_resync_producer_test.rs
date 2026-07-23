//! T081-T085：八个 Surface 都必须经 opaque resync -> 实际 storage/HTTP fact -> typed Frame。
//!
//! 这不是 projector fixture 测试：每个断言都先走 module command/correlation 再回放 PortReply，
//! 并对重复 resync 确认仍返回无 baseRevision 的完整 snapshot，防止已 detach renderer
//! 收到仅对内核旧 slot 有效的 replaceView patch。

mod common;
use common::*;

use bytes::Bytes;
use helix_core::Effect;
use helix_im::query::LocalStoreMode;
use serde_json::json;

const SURFACE_RESYNC: &str = "__helix_request_surface_snapshot";

fn corr_of(effect: &Effect) -> u64 {
    match effect {
        Effect::Persist { corr, .. } | Effect::Http { corr, .. } => corr.raw(),
        other => panic!("surface producer must be correlated storage/HTTP effect, got {other:?}"),
    }
}

fn frame_surfaces(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                .ok()
                .filter(|value| value["event"] == "im:interaction:frame"),
            _ => None,
        })
        .flat_map(|frame| {
            frame["data"]["mutations"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|mutation| mutation["surface"].as_str().map(str::to_string))
        .collect()
}

fn mutation(effects: &[Effect], surface: &str) -> serde_json::Value {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref()).ok()
            }
            _ => None,
        })
        .flat_map(|frame| {
            frame["data"]["mutations"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .find(|candidate| candidate["surface"] == surface)
        .unwrap_or_else(|| panic!("missing {surface} Frame mutation: {effects:?}"))
}

fn surface_command(surface: &str, scope: serde_json::Value) -> helix_core::Tick {
    command_json(
        SURFACE_RESYNC,
        json!({ "surface": surface, "scope": scope }),
    )
}

#[test]
fn dialog_unread_and_timeline_resync_replay_real_fact_producers() {
    let mut dialog = Harness::new_with_identity(&[], "viewer-1", "team-1", "", "", "");
    dialog.step(
        surface_command(
            "message.left-dialog-list",
            json!({ "kind":"user", "userId":"viewer-1", "teamId":"team-1" }),
        ),
        1,
    );
    let dialog_corr = corr_of(dialog.sink.as_slice().first().expect("dialog Scan"));
    dialog.step(
        port_reply_with_body(
            dialog_corr,
            Bytes::from(
                serde_json::to_vec(&json!([{
                    "id":"channel-dialog", "display_name":"Dialog", "type":"P",
                    "unread_count":2, "mention_count":1, "urgent_count":1, "last_post_at":10
                }]))
                .unwrap(),
            ),
        ),
        2,
    );
    assert_eq!(
        frame_surfaces(dialog.sink.as_slice()),
        vec![
            "message.left-dialog-list".to_string(),
            "message.unread-summary".to_string()
        ]
    );

    let mut timeline = Harness::new_with_store_mode(&[1], LocalStoreMode::Disabled);
    timeline.step(
        surface_command(
            "message.timeline",
            json!({ "kind":"timeline", "channelId":cid(1), "windowToken":"latest" }),
        ),
        3,
    );
    let timeline_corr = corr_of(timeline.sink.as_slice().first().expect("timeline HTTP"));
    let remote = json!({ "status":"SUCCESS", "data": { "posts": [{
        "id":"post-1", "temporaryId":"tmp-1", "channelId":cid(1), "userId":"author-1",
        "createAt":11, "type":"TEXT", "message":"hello", "props":{}
    }]}});
    timeline.step(
        port_reply_with_body(
            timeline_corr,
            enveloped(Bytes::from(serde_json::to_vec(&remote).unwrap())),
        ),
        4,
    );
    assert_eq!(
        frame_surfaces(timeline.sink.as_slice()),
        vec!["message.timeline"]
    );

    timeline.step(
        surface_command(
            "message.timeline",
            json!({ "kind":"timeline", "channelId":cid(1), "windowToken":"latest" }),
        ),
        5,
    );
    let timeline_patch_corr = corr_of(timeline.sink.as_slice().first().expect("timeline HTTP"));
    timeline.step(
        port_reply_with_body(
            timeline_patch_corr,
            enveloped(Bytes::from(serde_json::to_vec(&remote).unwrap())),
        ),
        6,
    );
    let timeline_replay = mutation(timeline.sink.as_slice(), "message.timeline");
    assert_eq!(timeline_replay["mutation"]["kind"], "timelineSnapshot");
    assert_eq!(timeline_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(timeline_replay["revision"], 2);
}

#[test]
fn timeline_resync_survives_a_newer_ordinary_query_generation() {
    let mut timeline = Harness::new_with_store_mode(&[1], LocalStoreMode::Disabled);
    let scope = json!({ "kind":"timeline", "channelId":cid(1), "windowToken":"latest" });
    let query = json!({ "channelId":cid(1), "windowToken":"latest" });

    timeline.step(surface_command("message.timeline", scope), 1);
    let resync_corr = corr_of(timeline.sink.as_slice().first().expect("resync HTTP"));

    timeline.step(command_json("im_query_messages_by_channel", query), 2);
    let ordinary_corr = corr_of(
        timeline
            .sink
            .as_slice()
            .first()
            .expect("newer ordinary query HTTP"),
    );

    let remote = json!({ "status":"SUCCESS", "data": { "posts": [{
        "id":"post-1", "temporaryId":"tmp-1", "channelId":cid(1), "userId":"author-1",
        "createAt":11, "type":"TEXT", "message":"hello", "props":{}
    }]}});
    let body = || enveloped(Bytes::from(serde_json::to_vec(&remote).unwrap()));

    timeline.step(port_reply_with_body(resync_corr, body()), 3);
    let recovery = mutation(timeline.sink.as_slice(), "message.timeline");
    assert_eq!(recovery["mutation"]["kind"], "timelineSnapshot");
    assert_eq!(recovery["baseRevision"], serde_json::Value::Null);
    assert_eq!(recovery["revision"], 1);

    timeline.step(port_reply_with_body(ordinary_corr, body()), 4);
    let continuous = mutation(timeline.sink.as_slice(), "message.timeline");
    assert_eq!(continuous["mutation"]["kind"], "timelinePatch");
    assert_eq!(continuous["baseRevision"], 1);
    assert_eq!(continuous["revision"], 2);
}

#[test]
fn channel_member_and_topic_explicit_resync_always_replays_complete_snapshots() {
    let mut h = Harness::new(&[]);
    let channel = "channel-storage";

    h.step(
        surface_command(
            "message.channel-summary",
            json!({ "kind":"channel", "channelId":channel }),
        ),
        1,
    );
    let summary_corr = corr_of(h.sink.as_slice().first().unwrap());
    let summary_row = Bytes::from(
        serde_json::to_vec(&json!([{
            "id":channel, "display_name":"Storage channel", "member_count":2, "updated_at":7
        }]))
        .unwrap(),
    );
    h.step(port_reply_with_body(summary_corr, summary_row.clone()), 2);
    assert_eq!(
        mutation(h.sink.as_slice(), "message.channel-summary")["mutation"]["kind"],
        "channelSummarySnapshot"
    );
    h.step(
        surface_command(
            "message.channel-summary",
            json!({ "kind":"channel", "channelId":channel }),
        ),
        3,
    );
    let summary_patch_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(port_reply_with_body(summary_patch_corr, summary_row), 4);
    let summary_replay = mutation(h.sink.as_slice(), "message.channel-summary");
    assert_eq!(summary_replay["mutation"]["kind"], "channelSummarySnapshot");
    assert_eq!(summary_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(summary_replay["revision"], 2);

    h.step(
        surface_command(
            "message.member-roster",
            json!({ "kind":"memberRoster", "channelId":channel, "cursor":"first" }),
        ),
        5,
    );
    let roster_corr = corr_of(h.sink.as_slice().first().unwrap());
    let roster_row = Bytes::from(
        serde_json::to_vec(&json!([
            {"channel_id":channel,"user_id":"member-1","user_name":"Ada","role":"ADMIN"}
        ]))
        .unwrap(),
    );
    h.step(port_reply_with_body(roster_corr, roster_row.clone()), 6);
    let roster = mutation(h.sink.as_slice(), "message.member-roster");
    assert_eq!(roster["mutation"]["kind"], "memberRosterSnapshot");
    assert_eq!(
        roster["mutation"]["view"]["items"][0]["actions"][0]["enabled"],
        false
    );
    assert_eq!(
        roster["mutation"]["view"]["items"][0]["actions"][0]["disabledReason"],
        "unsupportedByAuthorityV1"
    );
    h.step(
        surface_command(
            "message.member-roster",
            json!({ "kind":"memberRoster", "channelId":channel, "cursor":"first" }),
        ),
        7,
    );
    let roster_patch_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(port_reply_with_body(roster_patch_corr, roster_row), 8);
    let roster_replay = mutation(h.sink.as_slice(), "message.member-roster");
    assert_eq!(roster_replay["mutation"]["kind"], "memberRosterSnapshot");
    assert_eq!(roster_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(roster_replay["revision"], 2);

    let topic_row = Bytes::from(
        serde_json::to_vec(&json!([
            {"id":"topic-1","root_id":channel,"root_post_id":"root-1","display_name":"Roadmap","last_post_at":8}
        ]))
        .unwrap(),
    );
    h.step(
        surface_command(
            "message.topic-list",
            json!({ "kind":"topicList", "parentChannelId":channel, "cursor":"first" }),
        ),
        9,
    );
    let topic_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(port_reply_with_body(topic_corr, topic_row.clone()), 10);
    let topic = mutation(h.sink.as_slice(), "message.topic-list");
    assert_eq!(topic["mutation"]["kind"], "topicListSnapshot");
    assert_eq!(
        topic["mutation"]["view"]["items"][0]["actions"][0]["disabledReason"],
        "unsupportedByAuthorityV1"
    );
    h.step(
        surface_command(
            "message.topic-list",
            json!({ "kind":"topicList", "parentChannelId":channel, "cursor":"first" }),
        ),
        11,
    );
    let topic_patch_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(port_reply_with_body(topic_patch_corr, topic_row), 12);
    let topic_replay = mutation(h.sink.as_slice(), "message.topic-list");
    assert_eq!(topic_replay["mutation"]["kind"], "topicListSnapshot");
    assert_eq!(topic_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(topic_replay["revision"], 2);
}

#[test]
fn thread_and_receipt_explicit_resync_always_replays_complete_snapshots() {
    let mut h = Harness::new(&[]);
    let channel = "channel-http";
    h.step(
        surface_command(
            "message.thread-detail",
            json!({ "kind":"thread", "channelId":channel, "rootMessageId":"root-1" }),
        ),
        1,
    );
    let thread_corr = corr_of(h.sink.as_slice().first().unwrap());
    let thread_body = json!({ "status":"SUCCESS", "data": {
        "rootPost": {"id":"root-1","channelId":channel,"userId":"author-1","createAt":1,"type":"TEXT","message":"root","props":{}},
        "replies": [{"id":"reply-1","channelId":channel,"userId":"author-2","createAt":2,"type":"TEXT","message":"reply","props":{}}]
    }});
    h.step(
        port_reply_with_body(
            thread_corr,
            enveloped(Bytes::from(serde_json::to_vec(&thread_body).unwrap())),
        ),
        2,
    );
    assert_eq!(
        mutation(h.sink.as_slice(), "message.thread-detail")["mutation"]["kind"],
        "threadDetailSnapshot"
    );

    h.step(
        command_json(
            "im_get_reply_branch",
            json!({
                "channel_id": channel,
                "root_message_id": "root-1",
                "reply_first_level_id": "reply-1",
                "req_id": "branch-1",
                "page_number": 1,
                "page_size": 20
            }),
        ),
        3,
    );
    let branch_corr = corr_of(h.sink.as_slice().first().expect("branch HTTP"));
    let branch_body = json!({ "status":"SUCCESS", "data": {
        "data": [{
            "id":"reply-2", "channelId":channel, "userId":"author-3", "createAt":3,
            "type":"TEXT", "message":"nested", "props":{}, "replyId":"reply-1",
            "replyRootId":"root-1", "replyFirstLevelId":"reply-1"
        }],
        "hasMore": false
    }});
    h.step(
        port_reply_with_body(
            branch_corr,
            enveloped(Bytes::from(serde_json::to_vec(&branch_body).unwrap())),
        ),
        4,
    );
    let branch = mutation(h.sink.as_slice(), "message.thread-detail");
    assert_eq!(branch["mutation"]["kind"], "threadDetailPatch");
    assert_eq!(branch["baseRevision"], 1);
    assert_eq!(
        branch["mutation"]["patch"]["view"]["orderedReplyIds"],
        json!(["reply-1", "reply-2"])
    );

    h.step(
        surface_command(
            "message.thread-detail",
            json!({ "kind":"thread", "channelId":channel, "rootMessageId":"root-1" }),
        ),
        5,
    );
    let thread_patch_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(
        port_reply_with_body(
            thread_patch_corr,
            enveloped(Bytes::from(serde_json::to_vec(&thread_body).unwrap())),
        ),
        4,
    );
    let thread_replay = mutation(h.sink.as_slice(), "message.thread-detail");
    assert_eq!(thread_replay["mutation"]["kind"], "threadDetailSnapshot");
    assert_eq!(thread_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(thread_replay["revision"], 3);

    h.step(
        surface_command(
            "message.participant-receipts",
            json!({ "kind":"participantReceipts", "channelId":channel, "messageId":"root-1", "receiptKind":"read" }),
        ),
        5,
    );
    let receipt_corr = corr_of(h.sink.as_slice().first().unwrap());
    let receipt_body = json!({ "status":"SUCCESS", "data":[{
        "postId":"root-1", "authorUserId":"author-1", "snapshotId":"snapshot-1", "snapshotVersion":1,
        "memberIds":["author-1","reader-1"], "readBits":"01"
    }]});
    h.step(
        port_reply_with_body(
            receipt_corr,
            enveloped(Bytes::from(serde_json::to_vec(&receipt_body).unwrap())),
        ),
        6,
    );
    assert_eq!(
        mutation(h.sink.as_slice(), "message.participant-receipts")["mutation"]["kind"],
        "participantReceiptsSnapshot"
    );

    h.step(
        surface_command(
            "message.participant-receipts",
            json!({ "kind":"participantReceipts", "channelId":channel, "messageId":"root-1", "receiptKind":"read" }),
        ),
        7,
    );
    let receipt_patch_corr = corr_of(h.sink.as_slice().first().unwrap());
    h.step(
        port_reply_with_body(
            receipt_patch_corr,
            enveloped(Bytes::from(serde_json::to_vec(&receipt_body).unwrap())),
        ),
        8,
    );
    let receipt_replay = mutation(h.sink.as_slice(), "message.participant-receipts");
    assert_eq!(
        receipt_replay["mutation"]["kind"],
        "participantReceiptsSnapshot"
    );
    assert_eq!(receipt_replay["baseRevision"], serde_json::Value::Null);
    assert_eq!(receipt_replay["revision"], 2);
}
