//! DialogPatch 写后投影组装。
//!
//! 这里不直接 I/O：只生成 `StorageOp` 与从 driver 回包 bytes 组装 render-ready emit。

use crate::state::ChannelId;
use crate::sync_session::PostFields;
use helix_core::effect::{Row, SqlValue, StorageOp};
use helix_core::Effect;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDialogPatch {
    pub channel_id: ChannelId,
    pub event_seq: u64,
    pub msg_id: String,
    pub fields: PostFields,
    pub update: crate::channel_write::PostChannelUpdate,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogPatchProjection {
    pub unread_count: i64,
    pub mention_count: i64,
    pub urgent_count: i64,
    pub mention_list: Vec<String>,
    pub urgent_post_list: Vec<String>,
    pub unread_post_id: Option<String>,
    pub last_root_post_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberDialogProjection {
    pub user_id: String,
    pub unread_count: Option<i64>,
    pub unread_post_id: Option<String>,
    pub last_post: Option<Value>,
    pub last_post_at: Option<i64>,
    pub last_root_post_at: Option<i64>,
    pub mention_count: Option<i64>,
    pub mention_count_root: Option<i64>,
    pub mention_list: Option<Vec<String>>,
    pub mention_user: Option<String>,
    pub urgent_count: Option<i64>,
    pub urgent_post_list: Option<Vec<String>>,
    pub urgent_mention_user: Option<String>,
    pub msg_count: Option<i64>,
    pub msg_count_root: Option<i64>,
    pub msg_count_private: Option<i64>,
    pub last_read_seq: Option<i64>,
}

impl PendingDialogPatch {
    pub fn new(
        channel_id: ChannelId,
        event_seq: u64,
        msg_id: &str,
        fields: &PostFields,
        update: &crate::channel_write::PostChannelUpdate,
        source: &str,
    ) -> Self {
        Self {
            channel_id,
            event_seq,
            msg_id: msg_id.to_string(),
            fields: fields.clone(),
            update: update.clone(),
            source: source.to_string(),
        }
    }

    pub fn storage_ops(&self) -> Vec<StorageOp> {
        vec![
            crate::acl::to_effect::bump_channel_unread_op(&self.update),
            crate::acl::to_effect::get_channel_row_op(self.channel_id),
        ]
    }

    pub fn emit_from_channel_reply(&self, reply: &[u8]) -> Option<Effect> {
        let projection = projection_from_channel_reply(reply)?;
        Some(crate::acl::to_effect::emit_channel_update_by_post(
            self.channel_id,
            self.event_seq,
            self.msg_id.as_str(),
            &self.fields,
            &self.update,
            &projection,
            self.source.as_str(),
        ))
    }
}

pub fn member_dialog_from_update_channel(
    data: &Value,
    channel_id: ChannelId,
    auth_user_id: &str,
) -> Option<(Row, MemberDialogProjection)> {
    let channel = data.get("channel").unwrap_or(data);
    let user_id = pick_text(
        channel,
        data,
        &["userId", "user_id", "memberUserId", "member_user_id"],
    )
    .or_else(|| (!auth_user_id.is_empty()).then(|| auth_user_id.to_string()))?;

    let mut row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(channel_id.as_str().to_string()),
        ),
        ("user_id".to_string(), SqlValue::Text(user_id.clone())),
    ];

    let mut projection = MemberDialogProjection {
        user_id,
        unread_count: None,
        unread_post_id: None,
        last_post: None,
        last_post_at: None,
        last_root_post_at: None,
        mention_count: None,
        mention_count_root: None,
        mention_list: None,
        mention_user: None,
        urgent_count: None,
        urgent_post_list: None,
        urgent_mention_user: None,
        msg_count: None,
        msg_count_root: None,
        msg_count_private: None,
        last_read_seq: None,
    };

    if let Some(v) = pick_int(channel, data, &["unreadCount", "unread_count"]) {
        row.push(("unread_count".to_string(), SqlValue::Integer(v)));
        projection.unread_count = Some(v);
    }
    if let Some(v) = pick_text(channel, data, &["unreadPostId", "unread_post_id"]) {
        row.push(("unread_post_id".to_string(), SqlValue::Text(v.clone())));
        projection.unread_post_id = Some(v);
    }
    if let Some(v) = pick_value(channel, data, &["lastPost", "last_post"]) {
        row.push((
            "last_post".to_string(),
            SqlValue::Text(value_storage_text(v)),
        ));
        projection.last_post = Some(v.clone());
    }
    if let Some(v) = pick_int(channel, data, &["lastPostAt", "last_post_at"]) {
        row.push(("last_post_at".to_string(), SqlValue::Integer(v)));
        projection.last_post_at = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["lastRootPostAt", "last_root_post_at"]) {
        row.push(("last_root_post_at".to_string(), SqlValue::Integer(v)));
        projection.last_root_post_at = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["mentionCount", "mention_count"]) {
        row.push(("mention_count".to_string(), SqlValue::Integer(v)));
        projection.mention_count = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["mentionCountRoot", "mention_count_root"]) {
        row.push(("mention_count_root".to_string(), SqlValue::Integer(v)));
        projection.mention_count_root = Some(v);
    }
    if let Some(v) = pick_value(channel, data, &["mentionList", "mention_list"]) {
        row.push((
            "mention_list".to_string(),
            SqlValue::Text(value_storage_text(v)),
        ));
        projection.mention_list = Some(json_list_value(v));
    }
    if let Some(v) = pick_text(channel, data, &["mentionUser", "mention_user"]) {
        row.push(("mention_user".to_string(), SqlValue::Text(v.clone())));
        projection.mention_user = Some(v);
    }
    if let Some(v) = pick_int(
        channel,
        data,
        &[
            "urgentCount",
            "urgent_count",
            "urgentMentionCount",
            "urgent_mention_count",
        ],
    ) {
        row.push(("urgent_mention_count".to_string(), SqlValue::Integer(v)));
        projection.urgent_count = Some(v);
    }
    if let Some(v) = pick_value(channel, data, &["urgentPostList", "urgent_post_list"]) {
        row.push((
            "urgent_post_list".to_string(),
            SqlValue::Text(value_storage_text(v)),
        ));
        projection.urgent_post_list = Some(json_list_value(v));
    }
    if let Some(v) = pick_text(
        channel,
        data,
        &[
            "urgentMentionUser",
            "urgent_mention_user",
            "urgentCurrentName",
            "urgent_current_name",
        ],
    ) {
        row.push(("urgent_mention_user".to_string(), SqlValue::Text(v.clone())));
        projection.urgent_mention_user = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["msgCount", "msg_count"]) {
        row.push(("msg_count".to_string(), SqlValue::Integer(v)));
        projection.msg_count = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["msgCountRoot", "msg_count_root"]) {
        row.push(("msg_count_root".to_string(), SqlValue::Integer(v)));
        projection.msg_count_root = Some(v);
    }
    if let Some(v) = pick_int(channel, data, &["msgCountPrivate", "msg_count_private"]) {
        row.push(("msg_count_private".to_string(), SqlValue::Integer(v)));
        projection.msg_count_private = Some(v);
    }
    if let Some(v) = pick_int(
        channel,
        data,
        &[
            "lastReadSeq",
            "last_read_seq",
            "readLastSeq",
            "read_last_seq",
        ],
    ) {
        row.push(("last_read_seq".to_string(), SqlValue::Integer(v)));
        projection.last_read_seq = Some(v);
    }

    (row.len() > 2).then_some((row, projection))
}

pub fn projection_from_channel_reply(reply: &[u8]) -> Option<DialogPatchProjection> {
    let rows: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_slice(reply).ok()?;
    let row = rows.first()?;
    Some(DialogPatchProjection {
        unread_count: int_col(row, "unread_count").unwrap_or(0),
        mention_count: int_col(row, "mention_count").unwrap_or(0),
        urgent_count: int_col(row, "urgent_count").unwrap_or(0),
        mention_list: json_list_col(row, "mention_list"),
        urgent_post_list: json_list_col(row, "urgent_post_list"),
        unread_post_id: text_col(row, "unread_post_id").filter(|s| !s.is_empty()),
        last_root_post_at: int_col(row, "last_root_post_at").unwrap_or(0),
    })
}

pub fn channel_row_reply_from_row(row: &[(String, SqlValue)]) -> Vec<u8> {
    let obj = row
        .iter()
        .map(|(k, v)| {
            let value = match v {
                SqlValue::Text(s) => serde_json::Value::String(s.clone()),
                SqlValue::Integer(i) => serde_json::json!(i),
                SqlValue::Real(f) => serde_json::json!(f),
                SqlValue::Blob(b) => serde_json::json!(b),
                SqlValue::Null => serde_json::Value::Null,
            };
            (k.clone(), value)
        })
        .collect::<serde_json::Map<_, _>>();
    serde_json::to_vec(&vec![obj]).unwrap_or_default()
}

fn int_col(row: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<i64> {
    row.get(key).and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    })
}

fn pick_value<'a>(primary: &'a Value, fallback: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .find_map(|key| primary.get(*key).or_else(|| fallback.get(*key)))
}

fn pick_int(primary: &Value, fallback: &Value, keys: &[&str]) -> Option<i64> {
    pick_value(primary, fallback, keys).and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_u64().and_then(|n| i64::try_from(n).ok()))
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    })
}

fn pick_text(primary: &Value, fallback: &Value, keys: &[&str]) -> Option<String> {
    pick_value(primary, fallback, keys).and_then(|v| match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(_) | Value::Bool(_) => Some(v.to_string()),
        _ => None,
    })
}

fn value_storage_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn json_list_value(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        Value::Object(map) => map.keys().cloned().collect(),
        Value::String(s) => {
            if s.is_empty() {
                Vec::new()
            } else {
                serde_json::from_str::<Value>(s)
                    .ok()
                    .map(|v| json_list_value(&v))
                    .unwrap_or_else(|| vec![s.clone()])
            }
        }
        _ => Vec::new(),
    }
}

fn text_col(row: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    row.get(key).and_then(|v| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Null => None,
        other => Some(other.to_string()),
    })
}

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
