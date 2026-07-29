//! Read 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造 viewer 的频道已读绝对态。
pub fn channel(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:read:channel", data)
}

/// 从已持久化的当前成员绝对态构造 G-13 频道已读事件。
pub fn channel_from_member_projection(
    channel_id: &str,
    projection: &Value,
) -> Result<MessageV3Event, crate::ImError> {
    let row = projection.get("dialogPatch").unwrap_or(projection);
    channel(serde_json::json!({
        "channelId": channel_id,
        "userId": row.get("userId").cloned().unwrap_or(Value::Null),
        "state": "read",
        "unreadCount": row.get("unreadCount").cloned().unwrap_or(Value::Null),
        "unreadPostId": row.get("unreadPostId").cloned().unwrap_or(Value::Null),
        "mentionCount": row.get("mentionCount").cloned().unwrap_or(Value::Null),
        "mentionCountRoot": row.get("mentionCountRoot").cloned().unwrap_or(Value::Null),
        "mentionList": row.get("mentionList").cloned().unwrap_or(Value::Null),
        "urgentCount": row.get("urgentCount").cloned().unwrap_or(Value::Null),
        "urgentPostList": row.get("urgentPostList").cloned().unwrap_or(Value::Null),
        "lastReadSeq": row.get("lastReadSeq").cloned().unwrap_or(Value::Null),
        "projectionRevision": row.get("projectionRevision").cloned().unwrap_or(Value::Null),
        "lastPost": row.get("lastPost").cloned().unwrap_or(Value::Null),
        "lastPostAt": row.get("lastPostAt").cloned().unwrap_or(Value::Null),
        "lastRootPostAt": row.get("lastRootPostAt").cloned().unwrap_or(Value::Null),
        "msgCount": row.get("msgCount").cloned().unwrap_or(Value::Null),
        "msgCountRoot": row.get("msgCountRoot").cloned().unwrap_or(Value::Null),
        "msgCountPrivate": row.get("msgCountPrivate").cloned().unwrap_or(Value::Null),
    }))
}

/// 从 dialog 本地查询行恢复当前 viewer 的频道未读绝对态。
pub fn channels_from_storage_rows(
    reply_bytes: &[u8],
    viewer_user_id: &str,
) -> Result<Vec<MessageV3Event>, crate::ImError> {
    let rows: Vec<Value> = serde_json::from_slice(reply_bytes)
        .map_err(|error| crate::ImError::Parse(format!("dialog read rows: {error}")))?;
    rows.into_iter()
        .filter(|row| {
            row.get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty())
        })
        .map(|row| {
            let channel_id = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let unread_count = integer(&row, "unread_count", "unreadCount");
            channel(serde_json::json!({
                "channelId": channel_id,
                "userId": viewer_user_id,
                "state": if unread_count == 0 { "read" } else { "unread" },
                "unreadCount": unread_count,
                "mentionCount": integer(&row, "mention_count", "mentionCount"),
                "urgentCount": integer(&row, "urgent_count", "urgentCount"),
                "lastReadSeq": integer(&row, "last_read_seq", "lastReadSeq"),
                "projectionRevision": integer(&row, "projection_revision", "projectionRevision"),
            }))
        })
        .collect()
}

/// 读取 storage snake_case 或 wire camelCase 的整数字段。
fn integer(row: &Value, snake: &str, camel: &str) -> i64 {
    row.get(snake)
        .or_else(|| row.get(camel))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        })
        .unwrap_or_default()
}

/// 构造单条消息已读事实。
pub fn post(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:read", data)
}

/// 构造消息作者可见的 reader 列表。
pub fn post_readers(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:readers", data)
}

/// 按当前 viewer 与权威 reader 的关系构造 G-14 单视角终态。
pub fn post_for_viewer(
    channel_id: &str,
    post_id: &str,
    reader_id: &str,
    read_bits: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    if !reader_id.is_empty() && reader_id == viewer_user_id {
        return post(serde_json::json!({
            "id": post_id,
            "channelId": channel_id,
            "readBits": read_bits,
            "readerId": reader_id,
            "receiptRevision": receipt_revision,
        }));
    }
    post_readers(serde_json::json!({
        "id": post_id,
        "channelId": channel_id,
        "readBits": read_bits,
        "readerIds": if reader_id.is_empty() {
            Vec::<&str>::new()
        } else {
            vec![reader_id]
        },
        "receiptRevision": receipt_revision,
    }))
}
