//! 单条消息回执人员投影。
//!
//! Go 只提供消息发送时的成员快照顺序与 readBits；这里按相同下标生成已读/未读人员，
//! 并排除消息发送者。Angular 只消费下面的 render-ready 结果，不解释 bitmap。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};

pub(crate) fn emit(req_id: &str, body: &Value) -> Effect {
    let receipts: Vec<Value> = body
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let post_id = item.get("postId")?.as_str()?.trim();
            if post_id.is_empty() {
                return None;
            }
            let author_user_id = item.get("authorUserId")?.as_str()?.trim();
            let snapshot_id = item.get("snapshotId")?.as_str()?.trim();
            let snapshot_version = item.get("snapshotVersion").and_then(Value::as_i64)?;
            let read_bits = item.get("readBits")?.as_str()?;
            let member_ids: Vec<&str> = item
                .get("memberIds")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
                .collect();
            let valid = !author_user_id.is_empty()
                && !snapshot_id.is_empty()
                && snapshot_version > 0
                && read_bits.len() == member_ids.len()
                && read_bits.bytes().all(|bit| bit == b'0' || bit == b'1');
            if !valid {
                return Some(unavailable_receipt(post_id, snapshot_id, snapshot_version));
            }
            let mut reader_ids = Vec::with_capacity(member_ids.len());
            let mut unread_ids = Vec::with_capacity(member_ids.len());
            for (index, user_id) in member_ids.into_iter().enumerate() {
                if user_id == author_user_id {
                    continue;
                }
                if read_bits.as_bytes()[index] == b'1' {
                    reader_ids.push(user_id);
                } else {
                    unread_ids.push(user_id);
                }
            }
            let readers: Vec<Value> = reader_ids
                .iter()
                .map(|user_id| json!({ "userId": user_id }))
                .collect();
            let unreaders: Vec<Value> = unread_ids
                .iter()
                .map(|user_id| json!({ "userId": user_id }))
                .collect();
            Some(json!({
                "postId": post_id,
                "state": "ready",
                "snapshotId": snapshot_id,
                "snapshotVersion": snapshot_version,
                "readerCount": reader_ids.len(),
                "unreadCount": unread_ids.len(),
                "readerIds": reader_ids,
                "unreadIds": unread_ids,
                "readers": readers,
                "unreaders": unreaders,
            }))
        })
        .collect();
    emit_value(json!({ "reqId": req_id, "receipts": receipts }))
}

fn unavailable_receipt(post_id: &str, snapshot_id: &str, snapshot_version: i64) -> Value {
    json!({
        "postId": post_id,
        "state": "unavailable",
        "snapshotId": snapshot_id,
        "snapshotVersion": snapshot_version,
        "readerCount": 0,
        "unreadCount": 0,
        "readerIds": [],
        "unreadIds": [],
        "readers": [],
        "unreaders": [],
        "error": "member snapshot and readBits are inconsistent",
    })
}

pub(crate) fn emit_error(req_id: &str, error: &str) -> Effect {
    emit_value(json!({ "reqId": req_id, "receipts": [], "error": error }))
}

fn emit_value(data: Value) -> Effect {
    let payload = json!({ "event": "im:post:readers", "data": data });
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("static receipt projection must serialize"),
        )),
    }
}
