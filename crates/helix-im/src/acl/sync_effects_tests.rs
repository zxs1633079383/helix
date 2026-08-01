//! `sync_effects` 回归单测——锁定**离线 increment-replay 撤回**的投影发出口径
//! （UC-1.5-offline 四面 e2e ② 面的单元级护栏）。
//!
//! 背景：冷启动 cursor 落后 → `increment_channel_end` 触发 proactive resync（sync/notify HTTP）→
//! `handle_sync_reply` 调 `sync_mutation_emits` 逐事件吐投影。其中 `PostRevoke`（type=3）
//! **必须无条件** emit `im:post:revoke`（即便该 msg 不在 `messages` 内容快照里——撤回只依赖
//! msgId/channelId/eventSeq 与 revoke 终态）。
//!
//! 守 HX-C011 可证伪：每个「绿」断言配「破坏即红」对偶——
//!   - 撤回事件 → 必有 `im:post:revoke`（按 id/channelId/eventSeq 锚）；
//!   - 去掉撤回事件 → 必无 `im:post:revoke`（不会凭空发出）。

use super::{sync_mutation_emits, sync_mutation_emits_with_auth};
use crate::state::{ChannelId, Seq};
use crate::sync_session::{EventEnvelope, EventKind, PostFields};
use helix_core::Effect;
use std::collections::HashMap;

const CH: &str = "a9h5hrdsy3873dmg375a6ntqiw";
const MSG: &str = "g8wh1bx4mty47qhduyqhm4eaaa";

fn ch() -> ChannelId {
    ChannelId::from_str(CH).expect("test channel id is valid Id26")
}

fn revoke_ev(seq: u64) -> EventEnvelope {
    EventEnvelope::new(ch(), Seq(seq), EventKind::PostRevoke, PostFields::default())
        .with_msg_id(Some(MSG.to_string()))
}

fn upsert_ev(seq: u64) -> EventEnvelope {
    EventEnvelope::new(ch(), Seq(seq), EventKind::PostUpsert, PostFields::default())
        .with_msg_id(Some(MSG.to_string()))
}

fn read_ev(seq: u64, reader_id: &str) -> EventEnvelope {
    EventEnvelope::new(ch(), Seq(seq), EventKind::PostRead, PostFields::default())
        .with_msg_id(Some(MSG.to_string()))
        .with_event_identity(
            None,
            Some(reader_id.to_string()),
            1_700_000_123,
            String::new(),
        )
}

/// 把 Effect::Emit 解成 JSON（非 Emit → panic）。
fn emit_json(e: &Effect) -> serde_json::Value {
    match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
        other => panic!("expected Emit, got {other:?}"),
    }
}

/// 找第一个指定 event 名的投影 envelope（无 → None）。
fn find_emit<'a>(emits: &'a [Effect], event: &str) -> Option<serde_json::Value> {
    emits
        .iter()
        .map(emit_json)
        .find(|v| v.get("event").and_then(|e| e.as_str()) == Some(event))
}

/// 离线 replay 撤回不依赖 messages 快照，只发布最小 MessageV3 identity/state。
#[test]
fn revoke_replay_emits_minimal_post_revoke_even_without_messages() {
    let emits = sync_mutation_emits(&[revoke_ev(3)], &HashMap::new());
    assert_eq!(
        find_emit(&emits, "im:post:revoke"),
        Some(serde_json::json!({
            "event": "im:post:revoke",
            "data": {
                "id": MSG,
                "channelId": CH,
                "eventSeq": 3,
                "revoke": true
            }
        }))
    );
}

/// replay 窗仍保持 type1 received 先于 type3 revoke 的权威分桶顺序。
#[test]
fn upsert_then_revoke_emits_received_before_revoke() {
    let mut messages = HashMap::new();
    messages.insert(MSG.to_string(), PostFields::default());
    // 输入序与 sort_events_by_bucket 一致（type1 先于 type3）。
    let emits = sync_mutation_emits(&[upsert_ev(2), revoke_ev(3)], &messages);

    let events: Vec<String> = emits
        .iter()
        .map(|e| emit_json(e)["event"].as_str().unwrap_or("").to_string())
        .collect();
    let recv = events.iter().position(|e| e == "im:post:received");
    let revoke = events.iter().position(|e| e == "im:post:revoke");
    assert!(
        recv.is_some(),
        "PostUpsert(在 messages 内) 必产 im:post:received"
    );
    assert!(revoke.is_some(), "PostRevoke 必产 im:post:revoke");
    assert!(
        matches!((recv, revoke), (Some(received), Some(revoked)) if received < revoked),
        "received 必先于 revoke"
    );
}

/// 可证伪对偶：replay 窗内没有撤回事件时绝不凭空产出 revoke。
#[test]
fn no_revoke_event_means_no_post_revoke() {
    let mut messages = HashMap::new();
    messages.insert(MSG.to_string(), PostFields::default());
    let emits = sync_mutation_emits(&[upsert_ev(2)], &messages);
    assert!(
        find_emit(&emits, "im:post:revoke").is_none(),
        "无 PostRevoke 事件不得发出 im:post:revoke"
    );
}

/// UC13：离线 sender 只在 sync 同时给出 post、reader 与权威 readBits 时收到可定位回执。
#[test]
fn sync_read_with_authoritative_identity_and_bits_replays_post_read() {
    const READER: &str = "user-reader-678";
    let mut messages = HashMap::new();
    messages.insert(
        MSG.to_string(),
        PostFields {
            id: MSG.to_string(),
            read_bits: "1".to_string(),
            ..PostFields::default()
        },
    );

    let emits = sync_mutation_emits(&[read_ev(4, READER)], &messages);
    let receipt = find_emit(&emits, "im:post:read")
        .expect("权威 msgId + actorId + readBits 必须重放 im:post:read");
    let data = &receipt["data"];
    assert_eq!(data["msgId"], serde_json::json!(MSG));
    assert_eq!(data["readerId"], serde_json::json!(READER));
    assert_eq!(data["readBits"], serde_json::json!("1"));
    assert_eq!(data["receiptRevision"], serde_json::json!(1_700_000_123));
}

/// 已读 replay 的 fat 行仍属于原作者；viewer 身份只用于渲染本人态，不能在回执期间回退为他人。
#[test]
fn sync_read_keeps_authenticated_author_as_self_in_both_receipt_projections() {
    const AUTHOR: &str = "user-author-444";
    const READER: &str = "user-reader-678";
    let mut messages = HashMap::new();
    messages.insert(
        MSG.to_string(),
        PostFields {
            id: MSG.to_string(),
            user_id: AUTHOR.to_string(),
            read_bits: "1".to_string(),
            ..PostFields::default()
        },
    );

    let emits = sync_mutation_emits_with_auth(&[read_ev(4, READER)], &messages, AUTHOR);
    let read_echo = find_emit(&emits, "im:channel:read_echo")
        .expect("离线 read 必须保留 legacy channel:read_echo");
    let receipt = find_emit(&emits, "im:post:read").expect("权威 read 同时必须重放 sender receipt");

    for projection in [read_echo, receipt] {
        assert_eq!(projection["data"]["userId"], serde_json::json!(AUTHOR));
        assert_eq!(projection["data"]["isSelf"], serde_json::json!(true));
    }
}

/// 定向消息的隐藏成员不得从离线 read replay 得知 post identity 或回执状态。
#[test]
fn sync_read_skips_all_projections_for_hidden_viewer() {
    const AUTHOR: &str = "user-author-444";
    const VISIBLE: &str = "user-visible-678";
    const HIDDEN: &str = "user-hidden-999";
    let mut messages = HashMap::new();
    messages.insert(
        MSG.to_string(),
        PostFields {
            id: MSG.to_string(),
            user_id: AUTHOR.to_string(),
            msg_type: "TEXT".to_string(),
            viewers: vec![VISIBLE.to_string()],
            read_bits: "1".to_string(),
            ..PostFields::default()
        },
    );

    let emits = sync_mutation_emits_with_auth(&[read_ev(4, VISIBLE)], &messages, HIDDEN);
    assert!(
        emits.is_empty(),
        "隐藏成员不得收到定向消息的 read replay 投影"
    );
}

/// 缺消息快照时无法证明 viewer 权限，read replay 必须零投影而不是泄露 post identity。
#[test]
fn sync_read_without_message_snapshot_fails_closed() {
    let emits = sync_mutation_emits_with_auth(
        &[read_ev(4, "user-reader-678")],
        &HashMap::new(),
        "user-hidden-999",
    );

    assert!(
        emits.is_empty(),
        "缺消息快照的 read replay 必须 fail-closed"
    );
}

/// 可证伪对偶：actorId 或 readBits 任一缺失，Helix 不得猜 reader 或伪造离线回执。
#[test]
fn sync_read_without_authoritative_identity_or_bits_does_not_replay_post_read() {
    let mut messages_without_bits = HashMap::new();
    messages_without_bits.insert(MSG.to_string(), PostFields::default());
    let missing_bits =
        sync_mutation_emits(&[read_ev(4, "user-reader-678")], &messages_without_bits);
    assert!(find_emit(&missing_bits, "im:post:read").is_none());

    let mut messages = HashMap::new();
    messages.insert(
        MSG.to_string(),
        PostFields {
            read_bits: "1".to_string(),
            ..PostFields::default()
        },
    );
    let missing_actor = sync_mutation_emits(&[read_ev(4, "")], &messages);
    assert!(find_emit(&missing_actor, "im:post:read").is_none());
}
