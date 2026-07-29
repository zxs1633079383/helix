//! G-14a durable props row 到 MessageV3 模板确认绝对态的唯一转换。

use super::MessageV3Event;
use helix_core::effect::{Row, SqlValue};
use serde_json::Value;

/// 判断 props.template.userIds 是否为服务端声明的确认集合。
pub(super) fn has_confirmation(props: &str) -> bool {
    serde_json::from_str::<Value>(props)
        .ok()
        .is_some_and(|value| {
            value
                .get("template")
                .and_then(|template| template.get("userIds").or_else(|| template.get("user_ids")))
                .is_some_and(Value::is_array)
        })
}

/// 从 message row 的真实 props 列构造最小业务终态。
pub(super) fn from_row(row: &Row) -> Result<Option<MessageV3Event>, crate::ImError> {
    let id = text_column(row, "id");
    let channel_id = text_column(row, "channel_id");
    let event_seq = integer_column(row, "event_seq");
    let update_at = integer_column(row, "update_at");
    let props = text_column(row, "props")
        .and_then(|value| serde_json::from_str::<Value>(value).ok())
        .filter(|value| {
            value
                .get("template")
                .and_then(|template| template.get("userIds").or_else(|| template.get("user_ids")))
                .is_some_and(Value::is_array)
        });
    let (Some(id), Some(channel_id), Some(event_seq), Some(update_at), Some(props)) =
        (id, channel_id, event_seq, update_at, props)
    else {
        return Ok(None);
    };
    super::super::encode(
        "im:post:updated",
        serde_json::json!({
            "id": id,
            "channelId": channel_id,
            "eventSeq": event_seq,
            "updateAt": update_at,
            "props": props,
        }),
    )
    .map(Some)
}

/// 从 driver row 读取文本列，不接受隐式类型转换。
fn text_column<'a>(row: &'a Row, column: &str) -> Option<&'a str> {
    row.iter().find_map(|(name, value)| {
        (name == column)
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
    })
}

/// 从 driver row 读取整数列，不接受字符串数字别名。
fn integer_column(row: &Row, column: &str) -> Option<i64> {
    row.iter().find_map(|(name, value)| {
        (name == column)
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Integer(value) => Some(*value),
                _ => None,
            })
    })
}
