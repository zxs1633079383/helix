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
            // canonical: memberIds 已排作者；legacy: 作者恰好出现一次且 bitmap 与原序列等长。
            // 两者都在同一线性扫描中归一为纯接收者序列，保持服务端快照顺序。
            for (index, user_id) in member_ids.into_iter().enumerate() {
                if user_id == author_user_id {
                    continue;
                }
                let bit = read_bits.as_bytes()[index];
                recipient_user_ids.push(user_id);
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
                // INV-06：snapshotId 是权威键，channelId + createAt 仅在缺快照时兜底。
                "receiptKey": "snapshotId",
                "readerCount": reader_ids.len(),
                "unreadCount": unread_ids.len(),
                "recipientUserIds": recipient_user_ids,
                "authorExcluded": true,
                "contractVersion": 2,
                "readerIds": reader_ids.clone(),
                "unreadIds": unread_ids.clone(),
                // INV-07 canonical：Angular 只消费有序 readUserIds/unreadUserIds 与绝对计数。
                "readCount": reader_ids.len(),
                "allRead": unread_ids.is_empty(),
                "readUserIds": reader_ids,
                "unreadUserIds": unread_ids,
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
        "receiptKey": if snapshot_id.is_empty() { "channelIdCreateAtFallback" } else { "snapshotId" },
        "readerCount": 0,
        "unreadCount": 0,
        "recipientUserIds": [],
        "authorExcluded": true,
        "contractVersion": 2,
        "readCount": 0,
        "allRead": false,
        "readUserIds": [],
        "unreadUserIds": [],
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

/// 从 `authorUserId` / `createBy` 读消息作者；`userId` 在已读回执里是 reader 语义，不当作者。
fn author_of(object: &serde_json::Map<String, Value>) -> String {
    object
        .get("authorUserId")
        .or_else(|| object.get("createBy"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 读回执里的 reader（单视角终态用 `readerId`，wire 别名 `readerUserId`）。
fn reader_of(object: &serde_json::Map<String, Value>) -> String {
    object
        .get("readerId")
        .or_else(|| object.get("readerUserId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 把单条已读回执归一为 Angular 可直接渲染的绝对态（INV-06 / INV-07）。
///
/// - 返回 `false` 表示该回执必须被整条丢弃：当前 viewer 自己创建的消息的自读回执不产生业务事件
///   （T134 批量已读过滤在客户端侧的等价面）。
/// - `snapshotId` 存在即为权威键；否则只有 `channelId + createAt` 齐备时标记修复回退。
/// - 一旦携带 `memberIds` 有序快照，就在同一次 O(n) 扫描里剔除作者、生成有序
///   `readUserIds` / `unreadUserIds` 与绝对计数，并**删除外发 `readBits`**：
///   Angular 永不接触 bitmap。快照与 bitmap 不一致时 fail-closed 为 `unavailable`，
///   同样不外发 bitmap。
pub(crate) fn render_ready_post_read(data: &mut Value, viewer_user_id: &str) -> bool {
    let Some(object) = data.as_object_mut() else {
        return true;
    };
    let author_user_id = author_of(object);
    let reader_user_id = reader_of(object);
    if let Some(post_id) = object
        .get("postId")
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        object.insert("postId".to_string(), json!(post_id));
    }
    if !reader_user_id.is_empty() {
        object.insert("readerUserId".to_string(), json!(reader_user_id));
    }
    object.remove("id");
    object.remove("readerId");
    if !viewer_user_id.is_empty()
        && author_user_id == viewer_user_id
        && reader_user_id == viewer_user_id
    {
        return false;
    }

    let snapshot_id = object
        .get("snapshotId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if !snapshot_id.is_empty() {
        object.insert("snapshotId".to_string(), json!(snapshot_id));
        object.insert("receiptKey".to_string(), json!("snapshotId"));
    } else if object.contains_key("channelId") && object.contains_key("createAt") {
        object.insert("receiptKey".to_string(), json!("channelIdCreateAtFallback"));
    }

    let Some(member_values) = object.get("memberIds").and_then(Value::as_array).cloned() else {
        object.insert("state".to_string(), json!("unavailable"));
        object.insert("error".to_string(), json!("missing member snapshot"));
        object.insert("readCount".to_string(), json!(0));
        object.insert("unreadCount".to_string(), json!(0));
        object.insert("allRead".to_string(), json!(false));
        object.insert("readUserIds".to_string(), json!([]));
        object.insert("unreadUserIds".to_string(), json!([]));
        object.remove("readBits");
        object.remove("readerIds");
        object.remove("memberIds");
        return true;
    };
    let member_ids: Vec<String> = member_values
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
        .collect();
    let read_bits = object
        .get("readBits")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut seen = HashSet::with_capacity(member_ids.len());
    let consistent = member_ids.len() == member_values.len()
        && member_ids.iter().all(|id| seen.insert(id.clone()))
        && read_bits.len() == member_ids.len()
        && read_bits.bytes().all(|bit| bit == b'0' || bit == b'1')
        && member_ids
            .iter()
            .filter(|user_id| *user_id == &author_user_id)
            .count()
            <= 1;

    let mut read_user_ids: Vec<String> = Vec::with_capacity(member_ids.len());
    let mut unread_user_ids: Vec<String> = Vec::with_capacity(member_ids.len());
    if consistent {
        // INV-06：固定使用消息创建时的成员顺序，并剔除发送者——发送者不占 readBits。
        for (index, user_id) in member_ids.into_iter().enumerate() {
            if user_id == author_user_id {
                continue;
            }
            if read_bits.as_bytes()[index] == b'1' {
                read_user_ids.push(user_id);
            } else {
                unread_user_ids.push(user_id);
            }
        }
        object.insert("state".to_string(), json!("ready"));
    } else {
        object.insert("state".to_string(), json!("unavailable"));
        object.insert(
            "error".to_string(),
            json!("member snapshot and readBits are inconsistent"),
        );
    }
    object.insert("readCount".to_string(), json!(read_user_ids.len()));
    object.insert("unreadCount".to_string(), json!(unread_user_ids.len()));
    object.insert(
        "allRead".to_string(),
        json!(consistent && unread_user_ids.is_empty()),
    );
    object.insert("authorExcluded".to_string(), json!(true));
    object.insert("readUserIds".to_string(), json!(read_user_ids));
    object.insert("unreadUserIds".to_string(), json!(unread_user_ids));
    // INV-07：bitmap 与原始成员快照都不外发，Angular 只能消费上面的有序绝对态。
    object.remove("readBits");
    object.remove("readerIds");
    object.remove("memberIds");
    true
}

#[cfg(test)]
mod render_ready_post_read_tests {
    use super::render_ready_post_read;
    use serde_json::{json, Value};

    /// 构造带成员快照的单条已读回执 wire 形态。
    fn receipt(author: &str, reader: &str, members: Value, bits: &str) -> Value {
        json!({
            "id": "post-template-001",
            "channelId": "channel-phase1-001",
            "authorUserId": author,
            "readerId": reader,
            "snapshotId": "snapshot-post-template-001",
            "memberIds": members,
            "readBits": bits,
            "receiptRevision": 1_700_000_000_000_i64,
        })
    }

    #[test]
    fn ordered_lists_replace_bitmap_and_never_leak_read_bits() {
        let mut data = receipt(
            "user-actor-444",
            "user-actor-444",
            json!(["user-actor-444", "user-receiver-445", "user-receiver-446"]),
            "110",
        );
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
        let object = data.as_object().expect("receipt object");
        assert!(
            !object.contains_key("readBits"),
            "INV-07: 出站投影不得含 readBits，got {object:?}"
        );
        assert!(!object.contains_key("memberIds"));
        assert_eq!(data["readUserIds"], json!(["user-receiver-445"]));
        assert_eq!(data["unreadUserIds"], json!(["user-receiver-446"]));
        assert_eq!(data["readCount"], json!(1));
        assert_eq!(data["unreadCount"], json!(1));
        assert_eq!(data["allRead"], json!(false));
        assert_eq!(data["state"], json!("ready"));
    }

    #[test]
    fn author_never_occupies_a_read_bit_even_when_its_bit_is_set() {
        // 作者位是 1（发送者"已读"自己的消息），仍不得出现在任何一侧列表。
        let mut data = receipt(
            "user-actor-444",
            "user-receiver-445",
            json!(["user-actor-444", "user-receiver-445"]),
            "10",
        );
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
        assert_eq!(data["readUserIds"], json!([]));
        assert_eq!(data["unreadUserIds"], json!(["user-receiver-445"]));
        assert_eq!(data["readCount"], json!(0));
        assert_eq!(data["unreadCount"], json!(1));
        assert_eq!(data["authorExcluded"], json!(true));
    }

    #[test]
    fn snapshot_order_is_preserved_and_not_sorted() {
        let mut data = receipt(
            "user-actor-444",
            "user-receiver-446",
            json!(["user-receiver-446", "user-receiver-445"]),
            "10",
        );
        assert!(render_ready_post_read(&mut data, "user-actor-999"));
        assert_eq!(data["readUserIds"], json!(["user-receiver-446"]));
        assert_eq!(data["unreadUserIds"], json!(["user-receiver-445"]));
    }

    #[test]
    fn self_authored_self_read_receipt_is_dropped() {
        let mut data = receipt(
            "user-actor-444",
            "user-actor-444",
            json!(["user-actor-444", "user-receiver-445"]),
            "10",
        );
        assert!(
            !render_ready_post_read(&mut data, "user-actor-444"),
            "T134: 当前用户自己创建的消息的自读回执必须被过滤"
        );
    }

    #[test]
    fn other_viewer_keeps_receipt_for_the_same_authored_message() {
        let mut data = receipt(
            "user-actor-444",
            "user-actor-444",
            json!(["user-actor-444", "user-receiver-445"]),
            "10",
        );
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
    }

    #[test]
    fn snapshot_id_is_the_authoritative_receipt_key() {
        let mut data = receipt(
            "user-actor-444",
            "user-receiver-445",
            json!(["user-receiver-445"]),
            "1",
        );
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
        assert_eq!(data["receiptKey"], json!("snapshotId"));
        assert_eq!(data["snapshotId"], json!("snapshot-post-template-001"));
    }

    #[test]
    fn missing_snapshot_id_falls_back_to_channel_and_create_at() {
        let mut data = json!({
            "id": "post-template-001",
            "channelId": "channel-phase1-001",
            "createAt": 1_700_000_000_000_i64,
            "readerId": "user-receiver-445",
        });
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
        assert_eq!(data["receiptKey"], json!("channelIdCreateAtFallback"));
    }

    #[test]
    fn inconsistent_snapshot_fails_closed_without_leaking_bitmap() {
        let mut data = receipt(
            "user-actor-444",
            "user-receiver-445",
            json!(["user-actor-444", "user-receiver-445"]),
            "1",
        );
        assert!(render_ready_post_read(&mut data, "user-receiver-445"));
        assert_eq!(data["state"], json!("unavailable"));
        assert!(!data.as_object().expect("object").contains_key("readBits"));
        assert_eq!(data["readUserIds"], json!([]));
        assert_eq!(data["unreadUserIds"], json!([]));
        assert_eq!(data["allRead"], json!(false));
    }

    #[test]
    fn receipt_without_member_snapshot_fails_closed_without_bitmap() {
        // 旧 Go 节点缺 receiver snapshot 时不得把内部 bitmap 重新暴露给 Angular。
        let mut data = json!({
            "id": "post-template-001",
            "channelId": "channel-phase1-001",
            "readBits": "010",
            "readerId": "g14-reader",
            "receiptRevision": 1_700_001_400_020_i64,
        });
        assert!(render_ready_post_read(&mut data, "g14-author"));
        assert_eq!(data["state"], json!("unavailable"));
        assert_eq!(data["postId"], json!("post-template-001"));
        assert_eq!(data["readerUserId"], json!("g14-reader"));
        assert_eq!(data["readUserIds"], json!([]));
        assert_eq!(data["unreadUserIds"], json!([]));
        assert!(!data.as_object().expect("object").contains_key("readBits"));
    }
}
