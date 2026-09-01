//! Read 领域 MessageV3 事件。

use super::MessageV3Event;
use crate::error::ImError;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

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
    let rows = parse_dialog_rows(reply_bytes)?;
    channels_from_dialog_rows(&rows, viewer_user_id)
}

/// 将 dialog storage 回包解析为对象数组，供频道列表与成员回读共享严格边界。
pub(crate) fn parse_dialog_rows(reply_bytes: &[u8]) -> Result<Vec<Value>, crate::ImError> {
    serde_json::from_slice(reply_bytes)
        .map_err(|error| crate::ImError::Parse(format!("dialog read rows: {error}")))
}

/// 从已经验证的 dialog 行生成当前 viewer 的频道未读绝对态事件。
pub(crate) fn channels_from_dialog_rows(
    rows: &[Value],
    viewer_user_id: &str,
) -> Result<Vec<MessageV3Event>, crate::ImError> {
    rows.iter()
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

/// 合并首段 channel 行与当前 viewer 的 member 置顶回读，缺行或越权行一律失败闭合。
pub(crate) fn merge_dialog_rows_for_viewer(
    channel_rows: &[Value],
    member_reply_bytes: &[u8],
    auth_user_id: &str,
) -> Result<Vec<Value>, crate::ImError> {
    if auth_user_id.is_empty() {
        return Err(ImError::Parse(
            "dialog member readback missing auth_user_id".into(),
        ));
    }

    let mut channel_ids = HashSet::with_capacity(channel_rows.len());
    for row in channel_rows {
        let channel_id = dialog_row_channel_id(row, "channel")?;
        if !channel_ids.insert(channel_id) {
            return Err(ImError::Parse(format!(
                "dialog channel readback duplicate channel_id: {channel_id}"
            )));
        }
    }

    let member_rows = parse_dialog_rows(member_reply_bytes)?;
    let mut members_by_channel = HashMap::with_capacity(member_rows.len());
    for row in &member_rows {
        let user_id = dialog_row_text(row, "user_id", "userId", "member user_id")?;
        if user_id != auth_user_id {
            return Err(ImError::Parse(format!(
                "dialog member readback user scope mismatch: {user_id}"
            )));
        }
        let channel_id = dialog_row_channel_id(row, "member")?;
        dialog_row_bool(row, "channel_is_top", "channelIsTop")?;
        if members_by_channel
            .insert(channel_id.to_string(), row)
            .is_some()
        {
            return Err(ImError::Parse(format!(
                "dialog member readback duplicate channel_id: {channel_id}"
            )));
        }
    }

    let mut merged = Vec::with_capacity(channel_rows.len());
    for row in channel_rows {
        let channel_id = dialog_row_channel_id(row, "channel")?;
        let member = members_by_channel.get(channel_id).copied().ok_or_else(|| {
            ImError::Parse(format!(
                "dialog member readback missing channel_id: {channel_id}"
            ))
        })?;
        let top = dialog_row_bool(member, "channel_is_top", "channelIsTop")?;
        let mut row = row.clone();
        let object = row
            .as_object_mut()
            .ok_or_else(|| ImError::Parse("dialog channel row must be object".into()))?;
        let member_object = member
            .as_object()
            .ok_or_else(|| ImError::Parse("dialog member row must be object".into()))?;
        for (key, value) in member_object {
            if !matches!(
                key.as_str(),
                "id" | "channel_id" | "channelId" | "user_id" | "userId"
            ) {
                object.insert(key.clone(), value.clone());
            }
        }
        object.insert("channelIsTop".into(), Value::Bool(top));
        merged.push(row);
    }
    Ok(merged)
}

/// 提取 channel 或 channel_member 行的非空频道标识，并拒绝双字段不一致。
fn dialog_row_channel_id<'a>(row: &'a Value, scope: &str) -> Result<&'a str, ImError> {
    let id = row.get("id").and_then(Value::as_str);
    let channel_id = row.get("channel_id").and_then(Value::as_str);
    if let (Some(id), Some(channel_id)) = (id, channel_id) {
        if id != channel_id {
            return Err(ImError::Parse(format!(
                "dialog {scope} row channel id mismatch"
            )));
        }
    }
    id.or(channel_id)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("dialog {scope} row missing channel_id")))
}

/// 提取成员身份字段，兼容 storage snake_case 与协议 camelCase。
fn dialog_row_text<'a>(
    row: &'a Value,
    snake: &str,
    camel: &str,
    scope: &str,
) -> Result<&'a str, ImError> {
    row.get(snake)
        .or_else(|| row.get(camel))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("dialog {scope} missing text")))
}

/// 将成员置顶列解析为严格的布尔值，只接受 bool 或 SQLite 0/1。
fn dialog_row_bool(row: &Value, snake: &str, camel: &str) -> Result<bool, ImError> {
    let value = row
        .get(snake)
        .or_else(|| row.get(camel))
        .ok_or_else(|| ImError::Parse("dialog member row missing channel_is_top".into()))?;
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    match value.as_i64() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(ImError::Parse(
            "dialog member channel_is_top must be bool or 0/1".into(),
        )),
    }
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
    author_user_id: &str,
    reader_id: &str,
    snapshot_id: &str,
    member_ids: &[String],
    read_bits: &str,
    receipt_revision: i64,
    viewer_user_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    let data = serde_json::json!({
        "postId": post_id,
        "channelId": channel_id,
        "authorUserId": author_user_id,
        "readerUserId": reader_id,
        "snapshotId": snapshot_id,
        "memberIds": member_ids,
        "readBits": read_bits,
        "receiptRevision": receipt_revision,
    });
    if !reader_id.is_empty() && reader_id == viewer_user_id {
        return post(data);
    }
    post_readers(data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn dialog_merge_uses_member_unread_after_restart() {
        let channel_id = crate::state::test_channel_id(1);
        let channels = vec![serde_json::json!({
            "id": channel_id.as_str(),
            "unread_count": 7
        })];
        let members = serde_json::json!([{
            "channel_id": channel_id.as_str(),
            "user_id": "viewer",
            "channel_is_top": 0,
            "unread_count": 9,
            "projection_revision": 4
        }]);

        let merged = super::merge_dialog_rows_for_viewer(
            &channels,
            members.to_string().as_bytes(),
            "viewer",
        )
        .unwrap();

        assert_eq!(merged[0]["unread_count"], 9);
        assert_eq!(merged[0]["projection_revision"], 4);
    }
}
