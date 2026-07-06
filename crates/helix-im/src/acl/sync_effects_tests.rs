//! `sync_effects` 回归单测——锁定**离线 increment-replay 撤回**的投影发出口径
//! （UC-1.5-offline 四面 e2e ② 面的单元级护栏）。
//!
//! 背景：冷启动 cursor 落后 → `increment_channel_end` 触发 proactive resync（sync/notify HTTP）→
//! `handle_sync_reply` 调 `sync_mutation_emits` 逐事件吐投影。其中 `PostRevoke`（type=3）
//! **必须无条件** emit `im:post:deleted`（即便该 msg 不在 `messages` 内容快照里——撤回不依赖
//! body，用 `PostFields::default()` 填 fat MessageItemData）。
//!
//! 守 HX-C011 可证伪：每个「绿」断言配「破坏即红」对偶——
//!   - 撤回事件 → 必有 `im:post:deleted`（17 键 fat·按 msg_id/channel_id/event_seq 锚）；
//!   - 去掉撤回事件 → 必无 `im:post:deleted`（不会凭空发出）。

use super::sync_mutation_emits;
use crate::state::{ChannelId, Seq};
use crate::sync_session::{EventEnvelope, EventKind, PostFields};
use helix_core::Effect;
use std::collections::HashMap;

const CH: &str = "a9h5hrdsy3873dmg375a6ntqiw";
const MSG: &str = "g8wh1bx4mty47qhduyqhm4eaaa";

/// projection-schema §1 离线撤回 fat MessageItemData 的冻结 17 键集（与
/// `test/expect/uc-1.5-offline.expect.json` projection.dataKeys 同源·缺/多即偏离）。
const FAT_KEYS_17: &[&str] = &[
    "channel_id",
    "event_seq",
    "msg_id",
    "temporaryId",
    "channelId",
    "userId",
    "type",
    "message",
    "props",
    "createAt",
    "updateAt",
    "readBits",
    "viewers",
    "sendStatus",
    "reactions",
    "templateReceived",
    "systemNotice",
    "pinned",
];

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

/// 绿：离线 replay 撤回（msg 不在 messages 快照）→ 无条件 emit `im:post:deleted`，
/// 17 键 fat·按 msg_id/channel_id/event_seq 锚定。
#[test]
fn revoke_replay_emits_post_deleted_fat_17_keys_even_without_messages() {
    // messages 空：撤回不依赖 body（empty_fields 路径）——这正是离线 increment-replay 的真形态。
    let emits = sync_mutation_emits(&[revoke_ev(3)], &HashMap::new(), "");

    let deleted = find_emit(&emits, "im:post:deleted")
        .expect("PostRevoke 必产 im:post:deleted（断在 sync_mutation_emits gate→投影这跳）");
    let data = deleted
        .get("data")
        .and_then(|d| d.as_object())
        .expect("data 为对象");

    // 键集严格对齐冻结 17 键（多/缺即偏离 projection-schema）。
    let mut got: Vec<&str> = data.keys().map(String::as_str).collect();
    got.sort_unstable();
    let mut want: Vec<&str> = FAT_KEYS_17.to_vec();
    want.sort_unstable();
    assert_eq!(
        got, want,
        "im:post:deleted 必须是 17 键 fat MessageItemData"
    );

    // 领域锚：msg_id / channel_id / event_seq 对齐输入。
    assert_eq!(data["msg_id"], serde_json::json!(MSG));
    assert_eq!(data["channel_id"], serde_json::json!(CH));
    assert_eq!(data["event_seq"], serde_json::json!(3u64));
}

/// 绿 + 分桶序：replay 窗含 PostUpsert(载行) + PostRevoke(标撤回) → 先 received 后 deleted
/// （UC-1.5-offline「行先被 im:post:received 渲染再被 im:post:deleted 标记」的单元投影序）。
#[test]
fn upsert_then_revoke_emits_received_before_deleted() {
    let mut messages = HashMap::new();
    messages.insert(MSG.to_string(), PostFields::default());
    // 输入序与 sort_events_by_bucket 一致（type1 先于 type3）。
    let emits = sync_mutation_emits(&[upsert_ev(2), revoke_ev(3)], &messages, "");

    let events: Vec<String> = emits
        .iter()
        .map(|e| emit_json(e)["event"].as_str().unwrap_or("").to_string())
        .collect();
    let recv = events.iter().position(|e| e == "im:post:received");
    let del = events.iter().position(|e| e == "im:post:deleted");
    assert!(
        recv.is_some(),
        "PostUpsert(在 messages 内) 必产 im:post:received"
    );
    assert!(del.is_some(), "PostRevoke 必产 im:post:deleted");
    assert!(recv.unwrap() < del.unwrap(), "received 必先于 deleted");
}

/// 可证伪对偶（破坏即红）：replay 窗内**没有**撤回事件 → 绝不凭空产出 im:post:deleted。
#[test]
fn no_revoke_event_means_no_post_deleted() {
    let mut messages = HashMap::new();
    messages.insert(MSG.to_string(), PostFields::default());
    let emits = sync_mutation_emits(&[upsert_ev(2)], &messages, "");
    assert!(
        find_emit(&emits, "im:post:deleted").is_none(),
        "无 PostRevoke 事件不得发出 im:post:deleted（守可证伪·非 tautology）"
    );
}
