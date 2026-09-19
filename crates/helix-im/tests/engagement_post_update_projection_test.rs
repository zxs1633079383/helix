//! 投票/平均分 post_update 的 Helix 完整快照投影回归。

mod common;

use common::port_reply_with_body;
use common::{port_reply_ok, Harness};
use helix_core::effect::{Effect, Row, SqlValue};
use helix_core::port_codec::rows_to_reply_bytes;
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

/// 普通 post_update 遇到 cursor gap 时只能进入补齐编排，不能绕过原子屏障提前写库或发布。
#[test]
fn ordinary_post_update_gap_has_no_pre_persist_terminal_effects() {
    let mut harness = Harness::new_with_identity(&[1], "engagement-owner", "company-1", "", "", "");

    harness.step(vote_update_frame(2), 1_700_000_000_001);

    assert!(
        harness.sink.as_slice().iter().all(|effect| !matches!(
            effect,
            Effect::PersistFire { .. } | Effect::PersistAtomic { .. } | Effect::Emit { .. }
        )),
        "gapped edit must wait for missing authority and the atomic persist barrier"
    );
}

/// 缺口前置 post 提交后，buffer 中普通 edit 必须继续形成新的原子屏障而非 legacy fire-and-forget。
#[test]
fn ordinary_post_update_gap_releases_through_post_update_atomic() {
    let mut harness = Harness::new_with_identity(&[], "engagement-owner", "company-1", "", "", "");
    let channel_id = helix_im::state::ChannelId::from_str(&common::cid(1)).expect("channel id");
    harness.module.register_channel(channel_id, 0);
    harness.seed_channel_scope(&common::cid(1), "company-1", "engagement-owner");

    harness.step(vote_update_frame(2), 1_700_000_000_001);
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::PersistAtomic { .. } | Effect::Emit { .. })));

    harness.step(vote_create_frame(), 1_700_000_000_002);
    let create_corr = atomic_corr(&harness);
    harness.step(port_reply_ok(create_corr.raw()), 1_700_000_000_003);
    let edit_corr = atomic_corr(&harness);
    assert_ne!(edit_corr, create_corr);
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| { !matches!(effect, Effect::PersistFire { .. } | Effect::Emit { .. }) }));

    harness.step(port_reply_ok(edit_corr.raw()), 1_700_000_000_004);
    let event = first_event(&harness);
    assert_eq!(event["event"], "im:post:updated");
    assert_eq!(event["data"]["props"]["vote"]["joinCount"], 1);
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

/// 构造连续 `post` 入站帧，保留业务类型和完整 props 作为 authority 输入。
fn post_create_frame(post_type: &str, post_id: &str, props: Value) -> Tick {
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&json!({
            "action": "post",
            "data": {
                "id": post_id,
                "channelId": common::cid(1),
                "userId": "engagement-sender",
                "type": post_type,
                "message": "engagement post",
                "simpleMessage": "engagement post",
                "createAt": 1_700_000_000_000_i64,
                "updateAt": 1_700_000_000_001_i64,
                "props": props,
                "event_seq": 1
            }
        }))
        .expect("engagement post authority frame must serialize")
        .into(),
    ))
}

/// 在 `PersistAtomic` 成功后回灌 viewer 成员态，取得最终 `im:post:received` 投影。
fn post_create_projection(post_type: &str, post_id: &str, props: Value) -> Value {
    let mut harness = Harness::new_with_identity(&[1], "engagement-owner", "company-1", "", "", "");

    let initial = harness.step(
        post_create_frame(post_type, post_id, props),
        1_700_000_000_001,
    );
    let persist_corr = initial
        .iter()
        .find_map(|effect| match effect {
            common::EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("post create must begin with PersistAtomic");
    assert!(
        !initial
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::Emit)),
        "post create must wait for PersistAtomic success before emitting"
    );

    let committed = harness.step(port_reply_ok(persist_corr), 1_700_000_000_002);
    let readback_corr = committed
        .iter()
        .find_map(|effect| match effect {
            common::EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("post create PersistOk must schedule viewer member readback");
    assert!(
        !committed
            .iter()
            .any(|effect| matches!(effect, common::EffectKind::Emit)),
        "post create must wait for viewer readback before emitting"
    );

    let last_post = json!({
        "id": post_id,
        "userId": "engagement-sender",
        "message": "engagement post",
        "createAt": 1_700_000_000_000_i64
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(common::cid(1))),
        (
            "user_id".to_string(),
            SqlValue::Text("engagement-owner".to_string()),
        ),
        ("unread_count".to_string(), SqlValue::Integer(1)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(&[row])),
        1_700_000_000_003,
    );

    first_event(&harness)
}

/// `TASK_CARD` post 创建经原子提交后必须保留类型与任务卡 props。
#[test]
fn task_card_post_create_keeps_type_and_props_after_atomic_commit() {
    let props = json!({
        "taskCard": {
            "id": "task-1",
            "title": "处理告警",
            "priority": "high"
        }
    });
    let event = post_create_projection("TASK_CARD", "srveng00000000000000000001", props.clone());

    assert_eq!(event["event"], "im:post:received");
    assert_eq!(event["data"]["type"], "TASK_CARD");
    assert_eq!(event["data"]["props"], props);
}

/// `MEETING_NOTICE` post 创建经原子提交后必须保留类型与会议通知 props。
#[test]
fn meeting_notice_post_create_keeps_type_and_props_after_atomic_commit() {
    let props = json!({
        "meetingNotice": {
            "id": "meeting-1",
            "title": "周会提醒",
            "startAt": 1_700_000_360_000_i64
        }
    });
    let event = post_create_projection(
        "MEETING_NOTICE",
        "srveng00000000000000000002",
        props.clone(),
    );

    assert_eq!(event["event"], "im:post:received");
    assert_eq!(event["data"]["type"], "MEETING_NOTICE");
    assert_eq!(event["data"]["props"], props);
}

/// `MEETING_CARD` post 创建经原子提交后必须保留类型与会议卡 props。
#[test]
fn meeting_card_post_create_keeps_type_and_props_after_atomic_commit() {
    let props = json!({
        "meetingCard": {
            "id": "meeting-card-1",
            "title": "项目评审",
            "room": "A-101"
        }
    });
    let event = post_create_projection("MEETING_CARD", "srveng00000000000000000003", props.clone());

    assert_eq!(event["event"], "im:post:received");
    assert_eq!(event["data"]["type"], "MEETING_CARD");
    assert_eq!(event["data"]["props"], props);
}
