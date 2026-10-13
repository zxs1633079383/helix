//! ChannelUpdate 写后投影组装。
//!
//! 这里不直接 I/O：只生成 `StorageOp` 与从 driver 回包 bytes 组装 MessageV3 事件数据。

use crate::error::ImError;
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
    pub effect_id: Option<String>,
    /// The current viewer's notification mode from the notify-only WS member patch.
    pub notify: Option<String>,
    pub channel_is_top: Option<bool>,
    pub projection_revision: Option<u64>,
    pub unread_count: Option<i64>,
    pub reaction_unread_count: Option<i64>,
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

/// Require the monotonic identity shared by online patches and offline sync projections.
pub(crate) fn require_member_projection_identity(
    projection: &MemberChannelUpdate,
    context: &str,
) -> Result<(u64, String), ImError> {
    let revision = projection
        .projection_revision
        .filter(|value| *value > 0 && *value <= i64::MAX as u64)
        .ok_or_else(|| ImError::Parse(format!("{context} missing revision")))?;
    let effect_id = projection
        .effect_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ImError::Parse(format!("{context} missing effectId")))?
        .to_string();
    Ok((revision, effect_id))
}

/// 完整冷快照使用 Go canonical DTO；稀疏显示态不可借用它的版本水位。
pub(crate) fn complete_member_projection(
    data: &Value,
    channel_id: ChannelId,
    auth_user_id: &str,
    now_ms: u64,
) -> Option<(Row, MemberChannelUpdate)> {
    if auth_user_id.is_empty()
        || data["channelId"].as_str() != Some(channel_id.as_str())
        || data["userId"].as_str() != Some(auth_user_id)
        || !data["effectId"]
            .as_str()
            .is_some_and(|value| !value.trim().is_empty())
        || !data["projectionRevision"]
            .as_i64()
            .is_some_and(|value| value > 0)
    {
        return None;
    }
    for key in [
        "unreadCount",
        "reactionUnreadCount",
        "lastReadSeq",
        "mentionCount",
        "mentionCountRoot",
        "urgentCount",
        "msgCount",
        "msgCountRoot",
        "msgCountPrivate",
        "lastPostAt",
        "lastRootPostAt",
    ] {
        if !data[key].as_i64().is_some_and(|value| value >= 0) {
            return None;
        }
    }
    for key in [
        "unreadPostId",
        "lastPost",
        "mentionUser",
        "urgentMentionUser",
    ] {
        data[key].as_str()?;
    }
    for key in ["mentionList", "urgentPostList"] {
        if !data[key].as_array()?.iter().all(Value::is_string) {
            return None;
        }
    }
    if data["unreadCount"].as_i64()? > 0 && data["unreadPostId"].as_str()?.is_empty() {
        return None;
    }
    let (mut row, mut projection) =
        member_channel_from_update_channel(data, channel_id, auth_user_id, now_ms)?;
    if projection.unread_count == Some(0) {
        projection.unread_post_id = Some(String::new());
        if let Some((_, value)) = row.iter_mut().find(|(key, _)| key == "unread_post_id") {
            *value = SqlValue::Text(String::new());
        }
    }
    Some((row, projection))
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

/// 将 viewer-local `update_channel` 权威态转换为带确定性更新时间的成员投影。
pub fn member_channel_from_update_channel(
    data: &Value,
    channel_id: ChannelId,
    auth_user_id: &str,
    now_ms: u64,
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
        effect_id: pick_text(channel, data, &["effectId", "effect_id"]),
        notify: None,
        channel_is_top: None,
        projection_revision: projection_revision(channel, data),
        unread_count: None,
        reaction_unread_count: None,
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

    if let Some(revision) = projection.projection_revision {
        row.push((
            "projection_revision".to_string(),
            SqlValue::Integer(revision.min(i64::MAX as u64) as i64),
        ));
    }
    if let Some(effect_id) = projection.effect_id.as_ref() {
        row.push(("effect_id".to_string(), SqlValue::Text(effect_id.clone())));
    }

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
    if pick_value(
        channel,
        data,
        &["reactionUnreadCount", "reaction_unread_count"],
    )
    .is_some()
    {
        let count = pick_int(
            channel,
            data,
            &["reactionUnreadCount", "reaction_unread_count"],
        )?;
        if count < 0 {
            return None;
        }
        row.push((
            "reaction_unread_count".to_string(),
            SqlValue::Integer(count),
        ));
        projection.reaction_unread_count = Some(count);
    }
    // 与 increment 的 Go wire 对齐；空串是清除锚点，不能被通用 pick_text 当作缺失。
    if let Some(v) = pick_value(
        channel,
        data,
        &["unReadPostId", "unreadPostId", "unread_post_id"],
    )
    .and_then(Value::as_str)
    {
        row.push(("unread_post_id".to_string(), SqlValue::Text(v.to_owned())));
        projection.unread_post_id = Some(v.to_owned());
    }
    if let Some(v) = pick_value(channel, data, &["lastPost", "last_post"]) {
        let v = crate::message_summary::prepare_post(v);
        row.push((
            "last_post".to_string(),
            SqlValue::Text(value_storage_text(&v)),
        ));
        projection.last_post = Some(v);
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
    if let Some(v) = pick_value(channel, data, &["mentionUser", "mention_user"])
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
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
    if let Some(v) = pick_value(
        channel,
        data,
        &[
            "urgentMentionUser",
            "urgent_mention_user",
            "urgentCurrentName",
            "urgent_current_name",
        ],
    )
    .and_then(Value::as_str)
    .map(str::to_owned)
    {
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

    if row.len() <= 2 {
        return None;
    }
    row.push((
        "updated_at".to_string(),
        SqlValue::Integer(now_ms.min(i64::MAX as u64) as i64),
    ));
    Some((row, projection))
}

/// Decode the authoritative row returned by the final `ScopedGet` in a canonical
/// projection transaction. Emission must use this row, never the inbound frame.
pub fn member_channel_from_reply(
    reply: &[u8],
    channel_id: ChannelId,
    auth_user_id: &str,
    now_ms: u64,
) -> Option<MemberChannelUpdate> {
    let rows: Vec<Value> = serde_json::from_slice(reply).ok()?;
    let row = rows.first()?;
    member_channel_from_update_channel(row, channel_id, auth_user_id, now_ms)
        .map(|(_, projection)| projection)
}

/// A canonical retry may omit no absolute field it originally supplied. Compare
/// the supplied subset with the durable readback to detect same-revision conflicts.
pub fn member_projection_matches(
    expected: &MemberChannelUpdate,
    actual: &MemberChannelUpdate,
) -> bool {
    macro_rules! matches_optional {
        ($field:ident) => {
            expected.$field.is_none() || expected.$field == actual.$field
        };
    }
    expected.user_id == actual.user_id
        && matches_optional!(effect_id)
        && matches_optional!(notify)
        && matches_optional!(channel_is_top)
        && matches_optional!(projection_revision)
        && matches_optional!(unread_count)
        && matches_optional!(reaction_unread_count)
        && matches_optional!(unread_post_id)
        && matches_optional!(last_post)
        && matches_optional!(last_post_at)
        && matches_optional!(last_root_post_at)
        && matches_optional!(mention_count)
        && matches_optional!(mention_count_root)
        && matches_optional!(mention_list)
        && matches_optional!(mention_user)
        && matches_optional!(urgent_count)
        && matches_optional!(urgent_post_list)
        && matches_optional!(urgent_mention_user)
        && matches_optional!(msg_count)
        && matches_optional!(msg_count_root)
        && matches_optional!(msg_count_private)
        && matches_optional!(last_read_seq)
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
    const KEYS: &[&str] = &["projectionRevision", "projection_revision"];
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

    /// Reaction counts accept both transports, keep old sparse frames, and reject malformed authority.
    #[test]
    fn reaction_unread_count_wire_presence_and_validation() {
        let id = crate::state::test_channel_id(91);
        for key in ["reactionUnreadCount", "reaction_unread_count"] {
            for count in [0, 3] {
                let mut wire = serde_json::json!({"userId":"viewer","unreadCount":7});
                wire[key] = count.into();
                let (row, projection) =
                    member_channel_from_update_channel(&wire, id, "viewer", 1).unwrap();
                assert_eq!(projection.reaction_unread_count, Some(count));
                assert!(row
                    .iter()
                    .any(|(column, value)| column == "reaction_unread_count"
                        && matches!(value, SqlValue::Integer(actual) if *actual == count)));
            }
            for bad in [
                serde_json::json!(-1),
                serde_json::json!(1.5),
                Value::Null,
                serde_json::json!({}),
            ] {
                let mut wire = serde_json::json!({"userId":"viewer","unreadCount":7});
                wire[key] = bad;
                assert!(member_channel_from_update_channel(&wire, id, "viewer", 1).is_none());
            }
        }
        let (row, projection) = member_channel_from_update_channel(
            &serde_json::json!({"userId":"viewer","unreadCount":7}),
            id,
            "viewer",
            1,
        )
        .unwrap();
        assert_eq!(projection.reaction_unread_count, None);
        assert!(!row
            .iter()
            .any(|(column, _)| column == "reaction_unread_count"));
    }

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

        let (row, projection) = member_channel_from_update_channel(&data, channel_id, "", 90)
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
            92,
        )
        .expect("notify-only member patch should be persisted");
        assert!(row.iter().any(|(column, value)| {
            column == "notify" && matches!(value, SqlValue::Text(mode) if mode == "IGNORE")
        }));
        assert_eq!(projection.notify.as_deref(), Some("IGNORE"));
        assert_eq!(projection.unread_count, None);
        assert_eq!(projection.channel_is_top, None);
    }

    #[test]
    fn member_projection_revision_never_falls_back_to_event_or_time_fields() {
        let channel_id = crate::state::test_channel_id(93);
        let (_, legacy) = member_channel_from_update_channel(
            &serde_json::json!({
                "channelId": channel_id.as_str(),
                "userId": "viewer-93",
                "unreadCount": 1,
                "eventSeq": 77,
                "updateAt": 88
            }),
            channel_id,
            "viewer-93",
            93,
        )
        .expect("absolute unread is still a valid legacy patch");
        assert_eq!(legacy.projection_revision, None);

        let (_, canonical) = member_channel_from_update_channel(
            &serde_json::json!({
                "channelId": channel_id.as_str(),
                "userId": "viewer-93",
                "unreadCount": 1,
                "projectionRevision": 9,
                "effectId": "effect-9",
                "eventSeq": 77
            }),
            channel_id,
            "viewer-93",
            93,
        )
        .expect("canonical projection should parse");
        assert_eq!(canonical.projection_revision, Some(9));
        assert_eq!(canonical.effect_id.as_deref(), Some("effect-9"));
    }

    #[test]
    fn canonical_projection_detects_same_revision_payload_conflict() {
        let channel_id = crate::state::test_channel_id(94);
        let parse = |unread_count| {
            member_channel_from_update_channel(
                &serde_json::json!({
                    "channelId": channel_id.as_str(),
                    "userId": "viewer-94",
                    "projectionRevision": 9,
                    "effectId": "effect-9",
                    "unreadCount": unread_count
                }),
                channel_id,
                "viewer-94",
                94,
            )
            .unwrap()
            .1
        };

        assert!(member_projection_matches(&parse(1), &parse(1)));
        assert!(!member_projection_matches(&parse(2), &parse(1)));
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
