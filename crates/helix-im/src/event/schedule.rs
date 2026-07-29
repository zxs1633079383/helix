//! Schedule durable row 到 MessageV3 事件的唯一转换。

use super::MessageV3Event;
use helix_core::effect::{Row, SqlValue};

/// 从 creator 本地 channel_schedule row 构造 G-08 绝对态。
pub fn created_from_row(row: &Row) -> Result<Option<MessageV3Event>, crate::ImError> {
    let Some(data) = schedule_data(row, true) else {
        return Ok(None);
    };
    super::encode("im:channel:schedule-created", data).map(Some)
}

/// 从 creator 本地 channel_schedule row 构造 G-09 取消终态。
pub fn canceled_from_row(row: &Row) -> Result<Option<MessageV3Event>, crate::ImError> {
    let Some(data) = schedule_data(row, false) else {
        return Ok(None);
    };
    super::encode("im:channel:schedule-canceled", data).map(Some)
}

/// 将真实列映射为单份 schedule data，不制造别名或 action 状态。
fn schedule_data(row: &Row, active: bool) -> Option<serde_json::Value> {
    let channel_id = text_column(row, "channel_id")?;
    let status = text_column(row, "status")?;
    let revision = integer_column(row, "revision")?;
    let mut data = serde_json::json!({
        "channelId": channel_id,
        "hasSchedulePost": active,
        "status": status,
        "revision": revision,
    });
    if active {
        data["scheduleId"] = serde_json::json!(text_column(row, "schedule_id")?);
        data["ownerUserId"] = serde_json::json!(text_column(row, "owner_user_id")?);
        data["message"] = serde_json::json!(text_column(row, "message")?);
        data["messagePreview"] = serde_json::json!(text_column(row, "message_preview")?);
        data["sendAt"] = serde_json::json!(integer_column(row, "send_at")?);
    }
    Some(data)
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
