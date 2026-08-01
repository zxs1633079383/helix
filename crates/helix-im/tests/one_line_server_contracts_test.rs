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
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_create_posts",
            serde_json::json!({
                "req_id":"batch-1",
                "posts":[{"id":"p1"}],
                "channel_ids":[cid(1), cid(2)]
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

#[test]
fn relay_ids_are_loaded_from_local_store_before_create_posts() {
    let source_id = cid(10);
    let target_id = cid(20);
    let mut h = Harness::new_with_identity(&[], "444", "team-1", "破坏者", "研发", "平台");
    h.step(
        command_json(
            "im_create_posts",
            serde_json::json!({
                "req_id":"relay-1",
                "post_ids":[source_id],
                "target_channel_ids":[target_id],
                "mode":"individual"
            }),
        ),
        100,
    );
    let lookup_corr = persist_corr(&h);
    let row = serde_json::json!([{
        "id": source_id,
        "temporary_id":"source-tmp",
        "type":"TEXT",
        "message":"原消息",
        "simple_message":"原消息",
        "props":"{\"document\":{\"id\":\"d1\"}}",
        "create_at":90,
        "user_id":"678"
    }]);
    h.step(
        port_reply_with_body(lookup_corr, serde_json::to_vec(&row).unwrap().into()),
        101,
    );

    let body = h
        .last_http_body()
        .expect("local lookup must continue to createPosts");
    assert_eq!(body["channelIds"], serde_json::json!([target_id]));
    assert_eq!(body["posts"][0]["id"], "");
    assert_eq!(body["posts"][0]["userId"], "444");
    assert_eq!(body["posts"][0]["teamId"], "team-1");
    assert_eq!(body["posts"][0]["message"], "原消息");
    assert_eq!(body["posts"][0]["props"]["document"]["id"], "d1");
}

#[test]
fn relay_public_command_without_req_id_keeps_one_generated_id_through_batch_result() {
    let source_id = cid(11);
    let target_id = cid(21);
    let payload = serde_json::to_vec(&serde_json::json!({
        "post_ids":[source_id],
        "target_channel_ids":[target_id],
        "mode":"individual"
    }))
    .unwrap();
    let command = helix_im::client_api::build_command("im_relay_messages", &payload)
        .expect("public relay command must normalize to the canonical dispatch command");
    assert_eq!(command.name.as_ref(), "im_create_posts");

    let mut h = Harness::new_with_identity(&[], "444", "team-1", "破坏者", "研发", "平台");
    h.step(Tick::Command(command), 100);
    let lookup_corr = persist_corr(&h);
    let row = serde_json::json!([{
        "id":source_id,
        "temporary_id":"source-tmp",
        "type":"TEXT",
        "message":"原消息",
        "props":"{}",
        "create_at":90,
        "user_id":"678"
    }]);
    h.step(
        port_reply_with_body(lookup_corr, serde_json::to_vec(&row).unwrap().into()),
        101,
    );

    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status":"SUCCESS",
            "data":{
                "status":"success",
                "targets":[{"channelId":target_id,"status":"accepted"}]
            }
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 102);

    let data = &emits(&h, "im:posts:batch-result")[0];
    let req_id = data["reqId"]
        .as_str()
        .expect("Helix must expose the generated operation id for correlation");
    assert!(!req_id.is_empty());
    assert!(req_id.starts_with("relay-"));
    assert_eq!(data["targets"][0]["channelId"], target_id);
}

#[test]
fn relay_without_req_id_still_fails_closed_for_invalid_business_inputs() {
    let valid_post = cid(12);
    let valid_target = cid(22);
    let cases = [
        serde_json::json!({
            "post_ids":[],
            "target_channel_ids":[valid_target],
            "mode":"individual"
        }),
        serde_json::json!({
            "post_ids":[valid_post],
            "target_channel_ids":[],
            "mode":"individual"
        }),
        serde_json::json!({
            "post_ids":[valid_post],
            "target_channel_ids":["not-a-channel"],
            "mode":"individual"
        }),
        serde_json::json!({
            "post_ids":[valid_post],
            "target_channel_ids":[valid_target],
            "mode":"broadcast"
        }),
    ];

    for payload in cases {
        let bytes = serde_json::to_vec(&payload).unwrap();
        let command = helix_im::client_api::build_command("im_relay_messages", &bytes)
            .expect("client API normalization is not business validation");
        let mut h = Harness::new(&[]);
        let result = h.module.handle(&Tick::Command(command), 100, &mut h.sink);
        assert!(result.is_err(), "invalid relay input must fail: {payload}");
        assert!(
            h.sink.as_slice().is_empty(),
            "invalid relay input must not emit persistence or HTTP effects: {payload}"
        );
    }
}

#[test]
/// 验证合并转发在完成全部本地消息查询后只提交一个 MULTIPLY 请求。
fn relay_merged_mode_assembles_one_chat_record_after_all_local_lookups() {
    let source_a = cid(30);
    let source_b = cid(31);
    let source_channel = cid(20);
    let target = cid(40);
    let mut h = Harness::new_with_identity(&[], "444", "team-1", "破坏者", "研发", "平台");
    h.step(
        command_json(
            "im_create_posts",
            serde_json::json!({
                "req_id":"relay-merged",
                "post_ids":[source_a, source_b],
                "target_channel_ids":[target],
                "mode":"merged"
            }),
        ),
        100,
    );
    let first = persist_corr(&h);
    h.step(
        port_reply_with_body(
            first,
            serde_json::to_vec(&serde_json::json!([{
                "id":source_a,"temporary_id":"ta","channel_id":source_channel,"type":"TEXT","message":"A",
                "props":"{}","user_snapshot":"{\"userName\":\"A用户\"}","create_at":80
            }]))
            .unwrap()
            .into(),
        ),
        101,
    );
    let second = persist_corr(&h);
    h.step(
        port_reply_with_body(
            second,
            serde_json::to_vec(&serde_json::json!([{
                "id":source_b,"temporary_id":"tb","channel_id":source_channel,"type":"TEXT","message":"B",
                "props":"{}","user_snapshot":"{\"userName\":\"B用户\"}","create_at":90
            }]))
            .unwrap()
            .into(),
        ),
        102,
    );
    let source_channel_corr = persist_corr(&h);
    h.step(
        port_reply_with_body(
            source_channel_corr,
            serde_json::to_vec(&serde_json::json!([{
                "id":source_channel,"display_name":"来源群"
            }]))
            .unwrap()
            .into(),
        ),
        103,
    );

    let body = h.last_http_body().unwrap_or_else(|| {
        panic!(
            "all local rows must produce createPosts: {:?}",
            h.sink.as_slice()
        )
    });
    assert_eq!(body["posts"].as_array().unwrap().len(), 1);
    assert_eq!(body["posts"][0]["type"], "MULTIPLY");
    assert_eq!(body["posts"][0]["simpleMessage"], "[聊天记录]");
    let merged: serde_json::Value =
        serde_json::from_str(body["posts"][0]["props"]["mergeMessage"].as_str().unwrap()).unwrap();
    assert_eq!(merged.as_array().unwrap().len(), 2);
    assert_eq!(merged[0]["message"], "A");
    assert_eq!(merged[1]["message"], "B");
}

#[test]
fn post_read_list_derives_render_ready_read_and_unread_members_from_snapshot() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-1","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","authorUserId":"444","readBits":"101","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678","555"],"readerIds":["legacy-wrong"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let data = &emits(&h, "im:post:readers")[0];
    assert_eq!(data["reqId"], "receipt-1");
    assert_eq!(data["receipts"][0]["state"], "ready");
    assert_eq!(data["receipts"][0]["snapshotId"], "snapshot-1");
    assert_eq!(data["receipts"][0]["readerCount"], 1);
    assert_eq!(data["receipts"][0]["unreadCount"], 1);
    assert_eq!(
        data["receipts"][0]["recipientUserIds"],
        serde_json::json!(["678", "555"])
    );
    assert_eq!(
        data["receipts"][0]["readUserIds"],
        serde_json::json!(["555"])
    );
    assert_eq!(
        data["receipts"][0]["unreadUserIds"],
        serde_json::json!(["678"])
    );
    assert!(data["receipts"][0].get("readBits").is_none());
    assert_eq!(data["receipts"][0]["authorExcluded"], true);
    assert_eq!(data["receipts"][0]["contractVersion"], 2);
    assert_eq!(data["receipts"][0]["readers"][0]["userId"], "555");
    assert_eq!(data["receipts"][0]["unreaders"][0]["userId"], "678");
}

#[test]
fn post_read_list_accepts_canonical_snapshot_without_author() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-canonical","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","authorUserId":"444","readBits":"01","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["678","555"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0]["receipts"][0];
    assert_eq!(receipt["state"], "ready");
    assert_eq!(receipt["readerIds"], serde_json::json!(["555"]));
    assert_eq!(receipt["unreadIds"], serde_json::json!(["678"]));
    assert_eq!(receipt["readerCount"], 1);
    assert_eq!(receipt["unreadCount"], 1);
    assert_eq!(
        receipt["recipientUserIds"],
        serde_json::json!(["678", "555"])
    );
    assert_eq!(receipt["readUserIds"], serde_json::json!(["555"]));
    assert_eq!(receipt["unreadUserIds"], serde_json::json!(["678"]));
    assert!(receipt.get("readBits").is_none());
    assert_eq!(receipt["authorExcluded"], true);
    assert_eq!(receipt["contractVersion"], 2);
}

#[test]
fn post_read_list_rejects_repeated_author_and_illegal_bitmap() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-invalid-shapes","post_ids":["p1","p2"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","authorUserId":"444","readBits":"101","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678","444"]},{"postId":"p2","authorUserId":"444","readBits":"02","snapshotId":"snapshot-2","snapshotVersion":1002,"memberIds":["678","555"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let data = &emits(&h, "im:post:readers")[0];
    assert_eq!(data["receipts"][0]["state"], "unavailable");
    assert_eq!(data["receipts"][1]["state"], "unavailable");
    for receipt in data["receipts"].as_array().expect("receipts") {
        assert_eq!(receipt["recipientUserIds"], serde_json::json!([]));
        assert_eq!(receipt["readUserIds"], serde_json::json!([]));
        assert_eq!(receipt["unreadUserIds"], serde_json::json!([]));
        assert!(receipt.get("readBits").is_none());
        assert_eq!(receipt["authorExcluded"], true);
        assert_eq!(receipt["contractVersion"], 2);
    }
}

#[test]
fn post_read_list_fails_closed_when_snapshot_and_bitmap_do_not_match() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-invalid","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","authorUserId":"444","readBits":"1","snapshotId":"snapshot-1","snapshotVersion":1001,"memberIds":["444","678"]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0]["receipts"][0];
    assert_eq!(receipt["state"], "unavailable");
    assert_eq!(receipt["readerCount"], 0);
    assert_eq!(receipt["unreadCount"], 0);
    assert_eq!(receipt["recipientUserIds"], serde_json::json!([]));
    assert_eq!(receipt["readUserIds"], serde_json::json!([]));
    assert_eq!(receipt["unreadUserIds"], serde_json::json!([]));
    assert!(receipt.get("readBits").is_none());
    assert_eq!(receipt["authorExcluded"], true);
    assert_eq!(receipt["contractVersion"], 2);
}

#[test]
fn post_read_list_keeps_server_unavailable_row_when_bitmap_is_null() {
    let mut h = Harness::new(&[]);
    h.step(
        command_json(
            "im_post_read_list",
            serde_json::json!({"req_id":"receipt-unavailable","post_ids":["p1"]}),
        ),
        1,
    );
    let corr = http_corr(&h);
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"postId":"p1","authorUserId":"444","status":"unavailable","unavailableReason":"missing_member_snapshot","readBits":null,"snapshotVersion":0,"memberIds":[]}] }"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let receipt = &emits(&h, "im:post:readers")[0]["receipts"][0];
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
    let mut h = Harness::new_with_identity(&[], "444", "", "", "", "");
    h.step(
        inbound(serde_json::json!({
            "action":"increment_channel",
            "data":{"id":channel,"role":"MANAGER","needSync":false,"lastEventSeq":0}
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
