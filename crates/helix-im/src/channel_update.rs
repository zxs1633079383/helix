//! ChannelUpdate 写后投影组装。
//!
//! 这里不直接 I/O：只生成 `StorageOp` 与从 driver 回包 bytes 组装 MessageV3 事件数据。

use crate::state::ChannelId;
use crate::sync_session::PostFields;
use helix_core::effect::{Row, SqlValue, StorageOp};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingChannelUpdate {
    pub channel_id: ChannelId,
    pub event_seq: u64,
    pub msg_id: String,
    pub fields: PostFields,
    pub update: crate::channel_write::PostChannelUpdate,
    pub source: String,
    pub causation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelUpdateProjection {
    pub unread_count: i64,
    pub mention_count: i64,
    pub urgent_count: i64,
    pub mention_list: Vec<String>,
    pub urgent_post_list: Vec<String>,
    pub unread_post_id: Option<String>,
    pub last_root_post_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberChannelUpdate {
    pub user_id: String,
    /// The current viewer's notification mode from the notify-only WS member patch.
    pub notify: Option<String>,
    pub channel_is_top: Option<bool>,
    pub projection_revision: Option<u64>,
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

impl PendingChannelUpdate {
    /// 记录一次消息写入对频道绝对态的派生更新。
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
            causation_id: None,
        }
    }

    /// 绑定用于串联同步批次的因果标识。
    pub fn with_causation_id(mut self, causation_id: Option<String>) -> Self {
        self.causation_id = causation_id.filter(|value| !value.is_empty());
        self
    }

    /// 生成未读计数写入与权威频道行读回操作。
    pub fn storage_ops(&self) -> Vec<StorageOp> {
        vec![
            crate::acl::to_effect::bump_channel_unread_op(&self.update),
            crate::acl::to_effect::get_channel_row_op(self.channel_id),
        ]
    }

    /// 从已持久化频道行构造 `im:channel:update` 的绝对态数据。
    pub fn event_data_from_channel_reply(&self, reply: &[u8]) -> Option<Value> {
        let mut projection = projection_from_channel_reply(reply)?;
        projection.urgent_count = projection.urgent_count.max(0);
        Some(crate::acl::to_effect::post_channel_update_data(
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

pub fn member_channel_from_update_channel(
    data: &Value,
    channel_id: ChannelId,
    auth_user_id: &str,
) -> Option<(Row, MemberChannelUpdate)> {
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

    let mut projection = MemberChannelUpdate {
        user_id,
        notify: None,
        channel_is_top: None,
        projection_revision: projection_revision(channel, data),
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

    // `update_channel` carries this member-scoped value as a short scalar; keep it
    // separate from channel-wide fields so the caller can persist it on the composite key.
    if let Some(v) = pick_notify(channel, data) {
        row.push(("notify".to_string(), SqlValue::Text(v.clone())));
        projection.notify = Some(v);
    }

    if let Some(v) = pick_bool(channel, data, &["channelIsTop", "channel_is_top", "top"]) {
        row.push(("channel_is_top".to_string(), SqlValue::Integer(v as i64)));
        projection.channel_is_top = Some(v);
    }

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
        let v = v.max(0);
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

mod decode;
pub use decode::projection_from_channel_reply;

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

/// Read only the three server-defined notification modes; malformed values are ignored here
/// and rejected by the WS handler before any write effect is produced.
fn pick_notify(primary: &Value, fallback: &Value) -> Option<String> {
    pick_value(primary, fallback, &["notify"])
        .and_then(Value::as_str)
        .filter(|value| matches!(*value, "NORMAL" | "STRONG" | "IGNORE"))
        .map(str::to_string)
}

/// 只读取后端绝对态的布尔值；字符串等宽松形态交给上层 intent gate 拒绝。
fn pick_bool(primary: &Value, fallback: &Value, keys: &[&str]) -> Option<bool> {
    pick_value(primary, fallback, keys).and_then(Value::as_bool)
}

fn projection_revision(primary: &Value, fallback: &Value) -> Option<u64> {
    const KEYS: &[&str] = &[
        "lastEventSeq",
        "eventSeq",
        "event_seq",
        "updateAt",
        "update_at",
    ];
    [primary, fallback]
        .into_iter()
        .flat_map(|value| KEYS.iter().filter_map(|key| value.get(*key)))
        .filter_map(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
                .or_else(|| value.as_str().and_then(|number| number.parse::<u64>().ok()))
        })
        .max()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct the smallest post update needed to exercise channel read-back projection.
    fn pending_update() -> PendingChannelUpdate {
        let channel_id = crate::state::test_channel_id(91);
        let update = crate::channel_write::post_updates(
            channel_id,
            &serde_json::json!({"id":"post-91", "type":"NOTICE"}),
            "viewer-91",
            1,
        );
        PendingChannelUpdate::new(
            channel_id,
            1,
            "post-91",
            &PostFields::default(),
            &update,
            "test",
        )
    }

    /// A negative member urgent count is clamped before channel_member persistence and projection.
    #[test]
    fn clamps_negative_member_urgent_count_before_persist() {
        let channel_id = crate::state::test_channel_id(90);
        let data = serde_json::json!({
            "channel": {
                "userId": "viewer-90",
                "urgentCount": -4,
                "urgentPostList": ["post-90"]
            }
        });

        let (row, projection) = member_channel_from_update_channel(&data, channel_id, "")
            .expect("member update should contain a user and urgent projection");
        assert!(row.iter().any(|(column, value)| {
            column == "urgent_mention_count" && matches!(value, SqlValue::Integer(0))
        }));
        assert_eq!(projection.urgent_count, Some(0));
        assert_eq!(
            projection.urgent_post_list,
            Some(vec!["post-90".to_string()])
        );
    }

    /// A notify-only member patch produces a composite-key row without inventing badge fields.
    #[test]
    fn parses_notify_only_member_patch() {
        let channel_id = crate::state::test_channel_id(92);
        let (row, projection) = member_channel_from_update_channel(
            &serde_json::json!({"id": channel_id.as_str(), "userId": "viewer-92", "notify": "IGNORE"}),
            channel_id,
            "viewer-92",
        )
        .expect("notify-only member patch should be persisted");
        assert!(row.iter().any(|(column, value)| {
            column == "notify" && matches!(value, SqlValue::Text(mode) if mode == "IGNORE")
        }));
        assert_eq!(projection.notify.as_deref(), Some("IGNORE"));
        assert_eq!(projection.unread_count, None);
        assert_eq!(projection.channel_is_top, None);
    }

    /// A negative persisted urgent count is clamped before the channel update event is emitted.
    #[test]
    fn clamps_negative_readback_urgent_count_in_channel_event() {
        let pending = pending_update();
        let reply = br#"[{"unread_count":1,"mention_count":2,"urgent_count":-5,"urgent_post_list":"[\"post-91\"]","last_root_post_at":10}]"#;

        let data = pending
            .event_data_from_channel_reply(reply)
            .expect("valid channel readback should produce event data");
        assert_eq!(data["dialogPatch"]["urgentCount"], 0);
        assert_eq!(
            data["dialogPatch"]["urgentPostList"],
            serde_json::json!(["post-91"])
        );
    }
}
