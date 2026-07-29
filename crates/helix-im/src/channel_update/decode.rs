//! Dialog projection reply decoding.

use super::ChannelUpdateProjection;

pub fn projection_from_channel_reply(reply: &[u8]) -> Option<ChannelUpdateProjection> {
    let rows: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_slice(reply).ok()?;
    let row = rows.first()?;
    Some(ChannelUpdateProjection {
        unread_count: int_col(row, "unread_count").unwrap_or(0),
        mention_count: int_col(row, "mention_count").unwrap_or(0),
        urgent_count: int_col(row, "urgent_count").unwrap_or(0),
        mention_list: json_list_col(row, "mention_list"),
        urgent_post_list: json_list_col(row, "urgent_post_list"),
        unread_post_id: text_col(row, "unread_post_id").filter(|s| !s.is_empty()),
        last_root_post_at: int_col(row, "last_root_post_at").unwrap_or(0),
    })
}

/// 从存储读回中解析整型列，并兼容 SQLite 文本数字。
fn int_col(row: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key).and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    })
}

/// 从存储读回中解析文本列，非文本 JSON 保留其确定性序列化。
fn text_col(row: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key).and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    })
}

/// 从存储读回中解析字符串列表，并兼容历史 JSON 文本列。
fn json_list_col(row: &serde_json::Map<String, serde_json::Value>, key: &str) -> Vec<String> {
    match row.get(key) {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::String(s)) => serde_json::from_str::<Vec<String>>(s)
            .unwrap_or_else(|_| {
                if s.is_empty() {
                    Vec::new()
                } else {
                    vec![s.clone()]
                }
            }),
        _ => Vec::new(),
    }
}
