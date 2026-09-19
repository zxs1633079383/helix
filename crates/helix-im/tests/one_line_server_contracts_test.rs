//! Go 第二轮合同消费：G-05 batch、G-14 readers、G-16 viewer capabilities。

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::{Effect, Module, Tick};

fn inbound(value: serde_json::Value) -> Tick {
    Tick::Inbound(InboundBytes(serde_json::to_vec(&value).unwrap().into()))
}

fn http_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("command must emit HTTP")
}

fn persist_corr(h: &Harness) -> u64 {
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("command must emit local lookup")
}

fn emits(h: &Harness, event: &str) -> Vec<serde_json::Value> {
    h.sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event: bytes } => serde_json::from_slice(bytes.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|payload: &serde_json::Value| payload["event"] == event)
        .map(|payload| payload["data"].clone())
        .collect()
}

#[test]
fn create_posts_emits_partial_acceptance_without_claiming_delivery() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_create_posts",
            serde_json::json!({
                "req_id":"batch-1",
                "command_id":"stable-batch-1","mode":"item",
                "post_ids":["p1"],
                "target_channel_ids":[cid(1), cid(2)]
            }),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status":"SUCCESS",
            "data":{
                "status":"partial",
                "acceptedCount":1,
                "failedCount":1,
                "targets":[
                    {"channelId":cid(1),"status":"accepted"},
                    {"channelId":cid(2),"status":"failed","error":"rejected"}
                ]
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let data = &emits(&h, "im:posts:batch-result")[0];
    assert_eq!(data["reqId"], "batch-1");
    assert_eq!(data["batchStatus"], "partial");
    assert_eq!(data["targets"][0]["acceptanceStatus"], "accepted");
    assert_eq!(data["targets"][0]["deliveryStatus"], "pending");
    assert_eq!(data["deliveryAuthority"], "ws-post");
}

/// Forward commands use no local Get and never expose client-supplied content.
#[test]
fn relay_ids_go_to_server_authority_without_local_assembly() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(command_json("im_create_posts",serde_json::json!({"req_id":"r","command_id":"c","mode":"merge","post_ids":[cid(1),cid(2)],"target_channel_ids":[cid(3)]})),100);
    let body = h.last_http_body().unwrap();
    assert_eq!(body["postIds"], serde_json::json!([cid(1), cid(2)]));
    assert_eq!(body["mode"], "merged");
    assert!(body.get("posts").is_none());
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|e| matches!(e, Effect::Persist { .. })));
}

/// 验证服务端阅读快照按 readBits 生成当前渲染所需的已读与未读成员集合。
#[test]
fn post_read_list_derives_render_ready_read_and_unread_members_from_snapshot() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-1","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","channelId":"chfixx00000000000000000001","authorUserId":"444","readBits":"101","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678","555"],"readerIds":["legacy-wrong"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let data = &emits(&h, "im:post:readers")[0];
    assert_eq!(data["state"], "ready");
    assert_eq!(data["snapshotId"], "snapshot-1");
    assert_eq!(data["readCount"], 1);
    assert_eq!(data["unreadCount"], 1);
    assert_eq!(data["readUserIds"], serde_json::json!(["555"]));
    assert_eq!(data["unreadUserIds"], serde_json::json!(["678"]));
    assert!(data.get("readBits").is_none());
    assert_eq!(data["authorExcluded"], true);
}

#[test]
fn post_read_list_accepts_canonical_snapshot_without_author() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-canonical","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","channelId":"chfixx00000000000000000001","authorUserId":"444","readBits":"01","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["678","555"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0];
    assert_eq!(receipt["state"], "ready");
    assert_eq!(receipt["readCount"], 1);
    assert_eq!(receipt["unreadCount"], 1);
    assert_eq!(receipt["readUserIds"], serde_json::json!(["555"]));
    assert_eq!(receipt["unreadUserIds"], serde_json::json!(["678"]));
    assert!(receipt.get("readBits").is_none());
    assert_eq!(receipt["authorExcluded"], true);
}

#[test]
fn post_read_list_rejects_repeated_author_and_illegal_bitmap() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-invalid-shapes","post_ids":["p1","p2"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","channelId":"chfixx00000000000000000001","authorUserId":"444","readBits":"101","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678","444"]},{"postId":"p2","channelId":"chfixx00000000000000000001","authorUserId":"444","readBits":"02","snapshotId":"snapshot-2","snapshotVersion":1002,"memberIds":["678","555"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipts = emits(&h, "im:post:readers");
    assert_eq!(receipts[0]["state"], "unavailable");
    assert_eq!(receipts[1]["state"], "unavailable");
    for receipt in &receipts {
        assert_eq!(receipt["readUserIds"], serde_json::json!([]));
        assert_eq!(receipt["unreadUserIds"], serde_json::json!([]));
        assert!(receipt.get("readBits").is_none());
        assert_eq!(receipt["authorExcluded"], true);
    }
}

#[test]
fn post_read_list_fails_closed_when_snapshot_and_bitmap_do_not_match() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-invalid","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","channelId":"chfixx00000000000000000001","authorUserId":"444","readBits":"1","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0];
    assert_eq!(receipt["state"], "unavailable");
    assert_eq!(receipt["readCount"], 0);
    assert_eq!(receipt["unreadCount"], 0);
    assert_eq!(receipt["readUserIds"], serde_json::json!([]));
    assert_eq!(receipt["unreadUserIds"], serde_json::json!([]));
    assert!(receipt.get("readBits").is_none());
    assert_eq!(receipt["authorExcluded"], true);
}

#[test]
fn post_read_list_keeps_server_unavailable_row_when_bitmap_is_null() {
    let mut h = Harness::new_with_identity(&[1, 2], "444", "team-1", "", "", "");
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-unavailable","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","channelId":"chfixx00000000000000000001","authorUserId":"444","status":"unavailable","unavailableReason":"missing_member_snapshot","readBits":null,"snapshotVersion":0,"memberIds":[]}] }"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0];
    assert_eq!(receipt["postId"], "p1");
    assert_eq!(receipt["state"], "unavailable");
    assert_eq!(receipt["error"], "missing_member_snapshot");
}

#[test]
fn candidate_users_default_to_company_directory_and_emit_render_ready_projection() {
    let mut h = Harness::new_with_identity(&[], "444", "team-1", "破坏者", "研发", "平台");
    h.step(
        command_json(
            "im_user_candidates",
            serde_json::json!({"req_id":"candidate-1"}),
        ),
        1,
    );
    let request = h
        .last_http_body()
        .expect("candidate command must issue HTTP");
    assert_eq!(request["userIds"], serde_json::json!([]));
    assert_eq!(request["keyword"], "");
    assert_eq!(request["limit"], 50);

    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status":"SUCCESS",
            "data":[{
                "id":"678","username":"optimus","nickname":"擎天柱",
                "deptId":"d1","deptName":"总经办","orgId":"o1","orgName":"CEO"
            }]
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);
    let data = &emits(&h, "im:user:candidates")[0];
    assert_eq!(data["reqId"], "candidate-1");
    assert_eq!(data["users"][0]["userId"], "678");
    assert_eq!(data["users"][0]["displayName"], "擎天柱");
    assert_eq!(data["users"][0]["deptName"], "总经办");
    assert_eq!(data["users"][0]["orgName"], "CEO");
}

/// 设置事件只保留持久业务字段，不再派生 renderer capability 矩阵。
#[test]
fn settings_drop_viewer_capabilities_from_server_matrix() {
    let channel = cid(7);
    let mut h = Harness::new_with_identity(&[7], "444", "team-1", "", "", "");
    h.step(
        inbound(serde_json::json!({
            "action":"increment_channel",
            "data":{"id":channel,"teamId":"team-1","role":"MANAGER","needSync":false,"lastEventSeq":0}
        })),
        1,
    );
    h.step(
        inbound(serde_json::json!({
            "action":"update_channel",
            "data":{"channel":{
                "id":channel,
                "displayName":"新群名",
                "settingVersion":91,
                "capabilitiesByRole":{
                    "CREATOR":{"canCloseChannel":true,"canLeaveChannel":false,"canManageSettings":true,"canManageMembers":true},
                    "MANAGER":{"canCloseChannel":false,"canLeaveChannel":true,"canManageSettings":true,"canManageMembers":true},
                    "MEMBER":{"canCloseChannel":false,"canLeaveChannel":true,"canManageSettings":false,"canManageMembers":false}
                }
            }}
        })),
        2,
    );
    let corr = persist_corr(&h);
    h.step(port_reply_ok(corr), 3);

    let data = &emits(&h, "im:channel:settings-updated")[0];
    assert_eq!(data["settingVersion"], 91);
    assert_eq!(data["displayName"], "新群名");
    assert!(data.get("channelCapabilities").is_none());
    assert!(data.get("capabilitiesByRole").is_none());
}
