//! 单条消息回执人员投影。
//!
//! Go 只提供消息发送时的成员快照顺序与 readBits；这里按相同下标生成已读/未读人员，
//! 并排除消息发送者。Angular 只消费下面的 render-ready 结果，不解释 bitmap。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) fn emit(req_id: &str, body: &Value) -> Effect {
    let receipts = project(body);
    emit_value(json!({ "reqId": req_id, "receipts": receipts }))
}

pub(crate) fn project(body: &Value) -> Vec<Value> {
    body.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let post_id = item.get("postId")?.as_str()?.trim();
            if post_id.is_empty() {
                return None;
            }
            let snapshot_id = item
                .get("snapshotId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let snapshot_version = item
                .get("snapshotVersion")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if item.get("status").and_then(Value::as_str) == Some("unavailable") {
                let reason = item
                    .get("unavailableReason")
                    .and_then(Value::as_str)
                    .filter(|reason| !reason.trim().is_empty())
                    .unwrap_or("receipt unavailable");
                return Some(unavailable_receipt(
                    post_id,
                    snapshot_id,
                    snapshot_version,
                    reason,
                ));
            }
            let author_user_id = item.get("authorUserId")?.as_str()?.trim();
            let read_bits = item.get("readBits")?.as_str()?;
            let Some(member_values) = item.get("memberIds").and_then(Value::as_array) else {
                return Some(unavailable_receipt(
                    post_id,
                    snapshot_id,
                    snapshot_version,
                    "member snapshot and readBits are inconsistent",
                ));
            };
            let member_ids: Vec<&str> = member_values
                .iter()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
                .collect();
            let member_shape_valid = member_ids.len() == member_values.len();
            let mut seen = HashSet::with_capacity(member_ids.len());
            let unique = member_ids.iter().all(|id| seen.insert(*id));
            let author_count = member_ids
                .iter()
                .filter(|user_id| **user_id == author_user_id)
                .count();
            let valid = !author_user_id.is_empty()
                && !snapshot_id.is_empty()
                && snapshot_version > 0
                && member_shape_valid
                && unique
                && author_count <= 1
                && read_bits.len() == member_ids.len()
                && read_bits.bytes().all(|bit| bit == b'0' || bit == b'1');
            if !valid {
                return Some(unavailable_receipt(
                    post_id,
                    snapshot_id,
                    snapshot_version,
                    "member snapshot and readBits are inconsistent",
                ));
            }
            let mut reader_ids = Vec::with_capacity(member_ids.len());
            let mut unread_ids = Vec::with_capacity(member_ids.len());
            let mut recipient_user_ids = Vec::with_capacity(member_ids.len());
            let mut recipient_read_bits = String::with_capacity(member_ids.len());
            // canonical: memberIds 已排作者；legacy: 作者恰好出现一次且 bitmap 与原序列等长。
            // 两者都在同一线性扫描中归一为纯接收者序列，保持服务端快照顺序。
            for (index, user_id) in member_ids.into_iter().enumerate() {
                if user_id == author_user_id {
                    continue;
                }
                let bit = read_bits.as_bytes()[index];
                recipient_user_ids.push(user_id);
                recipient_read_bits.push(char::from(bit));
                if bit == b'1' {
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
                "recipientUserIds": recipient_user_ids,
                "readBits": recipient_read_bits,
                "authorExcluded": true,
                "contractVersion": 2,
                "readerIds": reader_ids,
                "unreadIds": unread_ids,
                "readers": readers,
                "unreaders": unreaders,
            }))
        })
        .collect()
}

fn unavailable_receipt(
    post_id: &str,
    snapshot_id: &str,
    snapshot_version: i64,
    error: &str,
) -> Value {
    json!({
        "postId": post_id,
        "state": "unavailable",
        "snapshotId": snapshot_id,
        "snapshotVersion": snapshot_version,
        "readerCount": 0,
        "unreadCount": 0,
        "recipientUserIds": [],
        "readBits": "",
        "authorExcluded": true,
        "contractVersion": 2,
        "readerIds": [],
        "unreadIds": [],
        "readers": [],
        "unreaders": [],
        "error": error,
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
