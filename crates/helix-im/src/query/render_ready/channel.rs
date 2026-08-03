//! Channel-list render-ready projection.
//!
//! Storage scans use the SQLite column names from `channel` (mostly snake_case),
//! while MessageV3 binds the camelCase channel contract. Keep this conversion at
//! the Helix boundary so every channel-list producer shares the same projection.

use serde_json::{json, Map, Value};

/// Pick the first channel field that exists, retaining its original JSON type.
fn pick(row: &Value, keys: &[&str]) -> Value {
    keys.iter()
        .find_map(|key| row.get(*key).cloned())
        .unwrap_or(Value::Null)
}

/// Parse a JSON-backed channel field without turning an invalid value into a raw string.
fn json_field(row: &Value, keys: &[&str], fallback: Value) -> Value {
    match pick(row, keys) {
        Value::String(raw) => serde_json::from_str::<Value>(&raw)
            .ok()
            .filter(|value| !value.is_null())
            .unwrap_or(fallback),
        Value::Null => fallback,
        value => value,
    }
}

/// Parse JSON-backed data but retain legitimate scalar channel values such as `IGNORE` or source names.
fn json_field_or_value(row: &Value, keys: &[&str], fallback: Value) -> Value {
    match pick(row, keys) {
        Value::String(raw) if raw.is_empty() => fallback,
        Value::String(raw) => {
            serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::String(raw))
        }
        Value::Null => fallback,
        value => value,
    }
}

/// Shape the persisted last-post object with the same message contract as the timeline.
fn shape_last_post(value: Value) -> Value {
    match value {
        Value::Object(_) => super::core::shape_row(&value, ""),
        _ => json!({}),
    }
}

/// Add one snake/camel alias to the render-ready channel object.
fn insert_alias(output: &mut Map<String, Value>, row: &Value, name: &str, keys: &[&str]) {
    let value = pick(row, keys);
    if !value.is_null() {
        output.insert(name.to_string(), value);
    }
}

/// Project one storage channel row into the MessageV3 channel-list contract.
pub(crate) fn shape_channel_row(row: &Value) -> Value {
    let Some(object) = row.as_object() else {
        return json!({});
    };
    let mut output = object.clone();

    insert_alias(&mut output, row, "id", &["id"]);
    insert_alias(
        &mut output,
        row,
        "displayName",
        &["display_name", "displayName"],
    );
    insert_alias(&mut output, row, "type", &["type"]);
    insert_alias(&mut output, row, "teamId", &["team_id", "teamId"]);
    insert_alias(&mut output, row, "userId", &["user_id", "userId"]);
    insert_alias(&mut output, row, "isActive", &["is_active", "isActive"]);
    insert_alias(
        &mut output,
        row,
        "unreadCount",
        &["unread_count", "unreadCount"],
    );
    insert_alias(
        &mut output,
        row,
        "unreadPostId",
        &["unread_post_id", "unreadPostId"],
    );
    insert_alias(
        &mut output,
        row,
        "mentionCount",
        &["mention_count", "mentionCount"],
    );
    insert_alias(
        &mut output,
        row,
        "mentionCountRoot",
        &["mention_count_root", "mentionCountRoot"],
    );
    insert_alias(
        &mut output,
        row,
        "mentionUser",
        &["mention_user", "mentionUser"],
    );
    insert_alias(
        &mut output,
        row,
        "urgentCount",
        &["urgent_count", "urgentCount"],
    );
    insert_alias(
        &mut output,
        row,
        "lastPostAt",
        &["last_post_at", "lastPostAt"],
    );
    insert_alias(
        &mut output,
        row,
        "lastRootPostAt",
        &["last_root_post_at", "lastRootPostAt"],
    );
    insert_alias(
        &mut output,
        row,
        "lastEventSeq",
        &["last_event_seq", "lastEventSeq"],
    );
    insert_alias(
        &mut output,
        row,
        "lastReadSeq",
        &["last_read_seq", "lastReadSeq"],
    );
    insert_alias(
        &mut output,
        row,
        "projectionRevision",
        &["projection_revision", "projectionRevision"],
    );
    insert_alias(&mut output, row, "topCount", &["top_count", "topCount"]);
    insert_alias(&mut output, row, "isRemove", &["is_remove", "isRemove"]);
    insert_alias(&mut output, row, "deleteAt", &["delete_at", "deleteAt"]);
    insert_alias(&mut output, row, "role", &["role"]);
    insert_alias(&mut output, row, "orient", &["orient"]);
    insert_alias(&mut output, row, "purpose", &["purpose"]);
    insert_alias(
        &mut output,
        row,
        "pictureType",
        &["picture_type", "pictureType"],
    );
    insert_alias(&mut output, row, "rootId", &["root_id", "rootId"]);
    insert_alias(
        &mut output,
        row,
        "rootPostId",
        &["root_post_id", "rootPostId"],
    );
    insert_alias(
        &mut output,
        row,
        "threadCount",
        &["thread_count", "threadCount"],
    );
    insert_alias(
        &mut output,
        row,
        "topicMsgCount",
        &["topic_msg_count", "topicMsgCount"],
    );
    insert_alias(
        &mut output,
        row,
        "adminMaxCount",
        &["admin_max_count", "adminMaxCount"],
    );
    insert_alias(
        &mut output,
        row,
        "hasUrgentPost",
        &["has_urgent_post", "hasUrgentPost"],
    );
    insert_alias(
        &mut output,
        row,
        "hasSchedulePost",
        &["has_schedule_post", "hasSchedulePost"],
    );
    insert_alias(&mut output, row, "createBy", &["create_by", "createBy"]);
    insert_alias(&mut output, row, "updateBy", &["update_by", "updateBy"]);
    insert_alias(
        &mut output,
        row,
        "urgentCurrentName",
        &["urgent_current_name", "urgentCurrentName"],
    );
    insert_alias(
        &mut output,
        row,
        "mentionPermission",
        &["mention_permission", "mentionPermission"],
    );
    insert_alias(
        &mut output,
        row,
        "noticePermission",
        &["notice_permission", "noticePermission"],
    );
    insert_alias(
        &mut output,
        row,
        "topPermission",
        &["top_permission", "topPermission"],
    );
    insert_alias(
        &mut output,
        row,
        "postMapDaily",
        &["post_map_daily", "postMapDaily"],
    );
    insert_alias(
        &mut output,
        row,
        "memberCount",
        &["member_count", "memberCount"],
    );
    insert_alias(
        &mut output,
        row,
        "subtopicsLoadedAt",
        &["subtopics_loaded_at", "subtopicsLoadedAt"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleId",
        &["schedule_id", "scheduleId"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleOwnerUserId",
        &["schedule_owner_user_id", "scheduleOwnerUserId"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleMessage",
        &["schedule_message", "scheduleMessage"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleMessagePreview",
        &["schedule_message_preview", "scheduleMessagePreview"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleSendAt",
        &["schedule_send_at", "scheduleSendAt"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleStatus",
        &["schedule_status", "scheduleStatus"],
    );
    insert_alias(
        &mut output,
        row,
        "scheduleRevision",
        &["schedule_revision", "scheduleRevision"],
    );
    insert_alias(
        &mut output,
        row,
        "channelIsTop",
        &["is_top", "channelIsTop"],
    );
    insert_alias(&mut output, row, "createAt", &["created_at", "createAt"]);
    insert_alias(&mut output, row, "updateAt", &["updated_at", "updateAt"]);

    output.insert(
        "lastPost".to_string(),
        shape_last_post(json_field(row, &["last_post", "lastPost"], json!({}))),
    );
    output.insert(
        "mentionList".to_string(),
        json_field(row, &["mention_list", "mentionList"], json!([])),
    );
    output.insert(
        "urgentPostList".to_string(),
        json_field(row, &["urgent_post_list", "urgentPostList"], json!([])),
    );
    output.insert(
        "members".to_string(),
        json_field(row, &["members"], json!([])),
    );
    output.insert(
        "adminUsers".to_string(),
        json_field(row, &["admin_users", "adminUsers"], json!([])),
    );
    output.insert("boss".to_string(), json_field(row, &["boss"], json!([])));
    output.insert("owner".to_string(), json_field(row, &["owner"], json!({})));
    output.insert(
        "notifyProps".to_string(),
        json_field_or_value(row, &["notify_props", "notifyProps"], json!({})),
    );
    output.insert("props".to_string(), json_field(row, &["props"], json!({})));
    output.insert(
        "source".to_string(),
        json_field_or_value(row, &["source"], json!({})),
    );
    output.insert(
        "picture".to_string(),
        json_field(row, &["picture"], json!({})),
    );
    output.insert(
        "targetUsers".to_string(),
        json_field(row, &["target_users", "targetUsers"], json!([])),
    );
    output.insert(
        "draft".to_string(),
        json_field(row, &["draft"], Value::Null),
    );
    Value::Object(output)
}

/// Project every object in a channel scan while retaining empty channels.
pub(crate) fn shape_channel_rows(rows: &Value) -> Value {
    Value::Array(
        rows.as_array()
            .into_iter()
            .flatten()
            .filter(|row| row.is_object())
            .map(shape_channel_row)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Channel scans expose the camel fields and a shaped last post to MessageV3.
    #[test]
    fn shapes_snake_channel_rows_without_dropping_empty_channels() {
        let rows = json!([
            {
                "id": "channel-1",
                "display_name": "测试群",
                "type": "P",
                "unread_count": 2,
                "mention_count": 1,
                "urgent_count": 3,
                "last_event_seq": 151,
                "last_read_seq": 9,
                "projection_revision": 12,
                "is_top": 1,
                "picture_type": "USER",
                "picture": "{\"userIds\":[\"444\"]}",
                "notify_props": "IGNORE",
                "source": "sync_events",
                "last_post": "{\"id\":\"post-1\",\"temporary_id\":\"tmp-1\",\"channel_id\":\"channel-1\",\"user_id\":\"444\",\"user_snapshot\":\"{\\\"userName\\\":\\\"破坏者\\\"}\",\"message\":\"回复消息\",\"type\":\"TEXT\"}"
            },
            { "id": "channel-2", "display_name": "空群" }
        ]);

        let shaped = shape_channel_rows(&rows);
        assert_eq!(shaped[0]["displayName"], "测试群");
        assert_eq!(shaped[0]["unreadCount"], 2);
        assert_eq!(shaped[0]["urgentCount"], 3);
        assert_eq!(shaped[0]["lastEventSeq"], 151);
        assert_eq!(shaped[0]["lastReadSeq"], 9);
        assert_eq!(shaped[0]["projectionRevision"], 12);
        assert_eq!(shaped[0]["channelIsTop"], 1);
        assert_eq!(shaped[0]["picture"]["userIds"][0], "444");
        assert_eq!(shaped[0]["notifyProps"], "IGNORE");
        assert_eq!(shaped[0]["source"], "sync_events");
        assert_eq!(shaped[0]["lastPost"]["message"], "回复消息");
        assert_eq!(shaped[1]["displayName"], "空群");
        assert_eq!(shaped.as_array().map(Vec::len), Some(2));
    }
}
