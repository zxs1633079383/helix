//! 批量消息回执人员投影。
//!
//! Go 只提供消息创建时的 receiver snapshot 顺序与内部 readBits；这里在 Helix 内部完成
//! O(n) 对齐后，向 Angular 只交付 snapshotId、有序用户 ID 与绝对计数。原始 bitmap、成员
//! 快照和兼容字段不跨越 `im:post:readers` 边界。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use serde_json::{json, Value};
use std::collections::HashSet;

/// 将批量 HTTP 回包拆成逐条 canonical 回执事件，避免向 Angular 泄漏查询聚合外壳。
pub(crate) fn emit(body: &Value) -> Vec<Effect> {
    project(body).into_iter().map(emit_value).collect()
}

/// 在线性扫描中把 Go 回执行转换为 Angular 可直接消费的绝对态。
pub(crate) fn project(body: &Value) -> Vec<Value> {
    let Some(rows) = body.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter().filter_map(project_row).collect()
}

/// 将一行 Go DTO 投影成严格 allowlist，损坏行只生成不可用终态。
fn project_row(item: &Value) -> Option<Value> {
    let post_id = item.get("postId")?.as_str()?.trim();
    if post_id.is_empty() {
        return None;
    }
    let channel_id = item
        .get("channelId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    let snapshot_id = item
        .get("snapshotId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if item.get("status").and_then(Value::as_str) == Some("unavailable") {
        let reason = item
            .get("unavailableReason")
            .and_then(Value::as_str)
            .filter(|reason| !reason.trim().is_empty())
            .unwrap_or("receipt unavailable");
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            reason,
        ));
    }

    let Some(author_user_id) = item.get("authorUserId").and_then(Value::as_str) else {
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            "receipt author is missing",
        ));
    };
    let Some(read_bits) = item.get("readBits").and_then(Value::as_str) else {
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            "member snapshot and readBits are inconsistent",
        ));
    };
    let Some(member_values) = item.get("memberIds").and_then(Value::as_array) else {
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            "member snapshot and readBits are inconsistent",
        ));
    };
    let member_ids: Option<Vec<&str>> = member_values
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|id| !id.is_empty() && id.trim() == *id)
        })
        .collect();
    let Some(member_ids) = member_ids else {
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            "member snapshot and readBits are inconsistent",
        ));
    };
    let mut seen = HashSet::with_capacity(member_ids.len());
    let unique = member_ids.iter().all(|id| seen.insert(*id));
    let author_user_id = author_user_id.trim();
    let author_count = member_ids
        .iter()
        .filter(|user_id| **user_id == author_user_id)
        .count();
    let valid = !channel_id.is_empty()
        && !snapshot_id.is_empty()
        && !author_user_id.is_empty()
        && unique
        && author_count <= 1
        && read_bits.len() == member_ids.len()
        && read_bits.bytes().all(|bit| bit == b'0' || bit == b'1');
    if !valid {
        return Some(unavailable_receipt(
            post_id,
            channel_id,
            snapshot_id,
            "member snapshot and readBits are inconsistent",
        ));
    }

    let mut read_user_ids = Vec::with_capacity(member_ids.len());
    let mut unread_user_ids = Vec::with_capacity(member_ids.len());
    // canonical: Go 已排作者；legacy 行若仍含作者，也在同一快照顺序中剔除。
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
    // 只允许公开合同字段；任何内部 DTO 字段都不能从输入对象复制到事件。
    let mut receipt = json!({
        "postId": post_id,
        "channelId": channel_id,
        "snapshotId": snapshot_id,
        "receiptKey": "snapshotId",
        "state": "ready",
        "readUserIds": read_user_ids,
        "unreadUserIds": unread_user_ids,
        "readCount": read_user_ids.len(),
        "unreadCount": unread_user_ids.len(),
        "allRead": unread_user_ids.is_empty(),
        "authorExcluded": true,
    });
    // 快照版本是服务端对账锚；缺失时保持兼容，但不伪造版本号。
    if let Some(snapshot_version) = item.get("snapshotVersion").and_then(Value::as_i64) {
        receipt["snapshotVersion"] = json!(snapshot_version);
    }
    Some(receipt)
}

/// 构造不泄漏 bitmap 的逐消息不可用终态，并保留诊断原因。
fn unavailable_receipt(post_id: &str, channel_id: &str, snapshot_id: &str, error: &str) -> Value {
    let receipt_key = if !snapshot_id.is_empty() {
        Some("snapshotId")
    } else if !channel_id.is_empty() {
        Some("channelIdCreateAtFallback")
    } else {
        None
    };
    let mut receipt = json!({
        "postId": post_id,
        "channelId": channel_id,
        "snapshotId": snapshot_id,
        "state": "unavailable",
        "error": error,
        "readUserIds": [],
        "unreadUserIds": [],
        "readCount": 0,
        "unreadCount": 0,
        "allRead": false,
        "authorExcluded": true,
    });
    // 诊断态也保留可对账的身份键，但不泄漏原始快照/bitmap。
    if let Some(receipt_key) = receipt_key {
        receipt["receiptKey"] = json!(receipt_key);
    }
    receipt
}

/// 将一条 render-ready 回执包装成冻结的 `im:post:readers` 事件。
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
