//! 投票/平均分 post_update 的 Helix 完整快照投影回归。

mod common;

use common::{port_reply_ok, Harness};
use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

/// 构造一个带完整参与集合的投票 post_update authority 帧。
fn vote_update_frame(seq: u64) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post_update",
            "data": {
                "id": "srveng00000000000000000001",
                "channelId": common::cid(1),
                "userId": "engagement-owner",
                "type": "VOTE",
                "message": "vote",
                "simpleMessage": "vote",
                "createAt": 1_700_000_000_000_i64,
                "updateAt": 1_700_000_000_001_i64,
                "props": {
                    "vote": {
                        "id": "vote-1",
                        "state": 0,
                        "options": [{"index": "1", "value": "A", "approvalCount": 1}],
                        "items": [{"userId": "engagement-owner", "indexes": ["1"]}],
                        "joinCount": 1
                    }
                },
                "quickReply": [],
                "event_seq": seq
            }
        }))
        .expect("engagement authority frame must serialize")
        .into(),
    ))
}

/// 构造真实顺序中的首条 vote post，作为 post_update 的 cursor 前置事件。
fn vote_create_frame() -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post",
            "data": {
                "id": "srveng00000000000000000001",
                "channelId": common::cid(1),
                "userId": "engagement-owner",
                "type": "VOTE",
                "message": "vote",
                "simpleMessage": "vote",
                "createAt": 1_700_000_000_000_i64,
                "updateAt": 1_700_000_000_000_i64,
                "props": {
                    "vote": {
                        "id": "vote-1",
                        "state": 0,
                        "options": [
                            {"index": "1", "value": "A", "approvalCount": 0}
                        ],
                        "items": [],
                        "joinCount": 0
                    }
                },
                "event_seq": 1
            }
        }))
        .expect("engagement post frame must serialize")
        .into(),
    ))
}

/// 构造带完整参与者集合和统计值的平均分 post_update authority 帧。
fn average_update_frame(seq: u64) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post_update",
            "data": {
                "id": "srvavg00000000000000000001",
                "channelId": common::cid(1),
                "userId": "engagement-owner",
                "type": "AVERAGE_SCORE",
                "message": "score",
                "simpleMessage": "score",
                "createAt": 1_700_000_000_000_i64,
                "updateAt": 1_700_000_000_001_i64,
                "props": {
                    "averageScore": {
                        "id": "average-1",
                        "state": 0,
                        "minScore": 0,
                        "maxScore": 100,
                        "participants": [{"userId": "member-1", "score": 4.5}],
                        "members": [{"userId": "member-1"}],
                        "totalCount": 1,
                        "averageScore": 4.5
                    }
                },
                "quickReply": [],
                "event_seq": seq
            }
        }))
        .expect("average authority frame must serialize")
        .into(),
    ))
}

/// 读取当前批次唯一的 post_update 原子提交关联。
fn atomic_corr(harness: &Harness) -> Correlation {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("post_update must persist content and cursor atomically")
}

/// 读取当前批次首个 DomainEvent。
fn first_event(harness: &Harness) -> Value {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .expect("PersistOk must emit a domain event")
}

/// post_update 成功投影必须把完整 vote.items 带到 Helix→客户端 DomainEvent，不能退化成稀疏状态。
#[test]
fn vote_post_update_keeps_full_items_snapshot_after_atomic_commit() {
    let mut harness = Harness::new_with_identity(&[1], "engagement-owner", "company-1", "", "", "");

    harness.step(vote_update_frame(1), 1_700_000_000_001);
    let corr = atomic_corr(&harness);
    harness.step(port_reply_ok(corr.raw()), 1_700_000_000_002);

    let event = first_event(&harness);
    assert_eq!(event["event"], "im:post:updated");
    assert_eq!(event["data"]["id"], "srveng00000000000000000001");
    assert_eq!(event["data"]["channelId"], common::cid(1));
    assert_eq!(event["data"]["props"]["vote"]["id"], "vote-1");
    assert_eq!(
        event["data"]["props"]["vote"]["items"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(event["data"]["props"]["vote"]["joinCount"], 1);
}

/// 首条 post 已先提交时，后续完整 post_update 仍必须覆盖旧的空集合，而不是被旧快照反写。
#[test]
fn vote_post_then_update_does_not_regress_to_initial_empty_snapshot() {
    let mut harness = Harness::new_with_identity(&[1], "engagement-owner", "company-1", "", "", "");

    harness.step(vote_create_frame(), 1_700_000_000_001);
    let create_corr = atomic_corr(&harness);
    harness.step(port_reply_ok(create_corr.raw()), 1_700_000_000_002);
    harness.step(vote_update_frame(2), 1_700_000_000_003);
    let update_corr = atomic_corr(&harness);
    harness.step(port_reply_ok(update_corr.raw()), 1_700_000_000_004);

    let event = first_event(&harness);
    assert_eq!(event["event"], "im:post:updated");
    assert_eq!(
        event["data"]["props"]["vote"]["items"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(event["data"]["props"]["vote"]["joinCount"], 1);
}

/// 平均分 post_update 成功投影必须保留 participants、members 和统计值的完整快照。
#[test]
fn average_score_post_update_keeps_full_participants_snapshot_after_atomic_commit() {
    let mut harness = Harness::new_with_identity(&[1], "engagement-owner", "company-1", "", "", "");

    harness.step(average_update_frame(1), 1_700_000_000_001);
    let corr = atomic_corr(&harness);
    harness.step(port_reply_ok(corr.raw()), 1_700_000_000_002);

    let event = first_event(&harness);
    assert_eq!(event["event"], "im:post:updated");
    assert_eq!(event["data"]["type"], "AVERAGE_SCORE");
    assert_eq!(event["data"]["props"]["averageScore"]["id"], "average-1");
    assert_eq!(
        event["data"]["props"]["averageScore"]["participants"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        event["data"]["props"]["averageScore"]["members"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(event["data"]["props"]["averageScore"]["totalCount"], 1);
    assert_eq!(event["data"]["props"]["averageScore"]["averageScore"], 4.5);
}
