//! DialogPatch 写后投影组装。
//!
//! 这里不直接 I/O：只生成 `StorageOp` 与从 driver 回包 bytes 组装 render-ready emit。

use crate::state::ChannelId;
use crate::sync_session::PostFields;
use helix_core::effect::{SqlValue, StorageOp};
use helix_core::Effect;

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
