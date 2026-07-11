//! Go 第二轮合同消费：G-05 batch、G-14 readers、G-16 viewer capabilities。

mod common;
use common::*;

use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};

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
fn post_read_list_emits_render_ready_readers_without_read_bits() {
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
        br#"{"status":"SUCCESS","data":[{"postId":"p1","readBits":{"Data":[128],"Len":2},"readerIds":["444","678"],"readers":[{"userId":"444"},{"userId":"678"}]}]}"#.to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 2);

    let data = &emits(&h, "im:post:readers")[0];
    assert_eq!(data["reqId"], "receipt-1");
    assert_eq!(data["receipts"][0]["readerCount"], 2);
    assert_eq!(data["receipts"][0]["readers"][1]["userId"], "678");
    assert!(data["receipts"][0].get("readBits").is_none());
}

#[test]
fn settings_select_viewer_capabilities_from_server_matrix() {
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

    let data = &emits(&h, "im:channel:settings-updated")[0];
    assert_eq!(data["settingVersion"], 91);
    assert_eq!(data["channelCapabilities"]["canManageSettings"], true);
    assert_eq!(data["channelCapabilities"]["canCloseChannel"], false);
}
