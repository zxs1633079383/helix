mod common;

use bytes::Bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Effect, Tick};
use serde_json::{json, Value};

use common::{cid, port_reply_ok, sid, Harness};

const VIEWER_USER_ID: &str = "678";

/// 构造当前 viewer 自己发送的 root 消息批量更新 authority。
fn posts_update_authority() -> Value {
    json!({
        "action": "posts_update",
        "data": {
            "events": [{
                "event_seq": 1,
                "post": {
                    "id": sid(1),
                    "temporaryId": "tmp-posts-update-viewer",
                    "channelId": cid(7),
                    "userId": VIEWER_USER_ID,
                    "type": "TEXT",
                    "message": "root message",
                    "simpleMessage": "root message",
                    "replyCount": 1,
                    "createAt": 1_700_000_000_001_u64,
                    "updateAt": 1_700_000_000_002_u64
                }
            }]
        }
    })
}

/// 读取 posts_update 原子提交 correlation，确保测试经过真实 PersistAtomic barrier。
fn atomic_corr(effects: &[Effect]) -> Correlation {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("posts_update must persist atomically")
}

/// 从提交后的 effect batch 读取指定业务事件。
fn emitted_event(effects: &[Effect], event_name: &str) -> Value {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .find(|event| event["event"] == event_name)
        .unwrap_or_else(|| panic!("missing emitted event {event_name}"))
}

/// 当前用户回复后收到 root posts_update 时，提交后的批量投影必须保持自端身份。
#[test]
fn posts_update_projects_root_message_for_current_viewer_after_commit() {
    let mut harness =
        Harness::new_with_identity(&[7], VIEWER_USER_ID, "company-1", "张无忌", "总经办", "CEO");
    let authority = posts_update_authority();

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).expect("posts_update json"),
        ))),
        1_700_000_000_003,
    );
    assert!(
        harness
            .sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Emit { .. })),
        "business event must wait for PersistAtomic success"
    );
    let corr = atomic_corr(harness.sink.as_slice());

    harness.step(port_reply_ok(corr.raw()), 1_700_000_000_004);
    let event = emitted_event(harness.sink.as_slice(), "im:post:batch-updated");
    let root = &event["data"]["posts"][0];

    assert_eq!(root["id"], sid(1));
    assert_eq!(root["userId"], VIEWER_USER_ID);
    assert_eq!(root["replyCount"], 1);
    assert_eq!(root["isSelf"], true);
}
