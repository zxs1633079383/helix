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

/// Keep persisted urgent counters non-negative while preserving ordinary JSON values.
fn non_negative_count(value: Value) -> Value {
    match &value {
        Value::Number(number) if number.as_f64().is_some_and(|value| value < 0.0) => Value::from(0),
        Value::String(raw) if raw.trim().parse::<f64>().is_ok_and(|value| value < 0.0) => {
            Value::from(0)
        }
        _ => value,
    }
}

/// Write the urgent counter under every existing alias so legacy rows cannot leak a negative value.
fn insert_non_negative_count_alias(
    output: &mut Map<String, Value>,
    row: &Value,
    name: &str,
    keys: &[&str],
) {
    let value = non_negative_count(pick(row, keys));
    if value.is_null() {
        return;
    }
    output.insert(name.to_string(), value.clone());
    for key in keys {
        if output.contains_key(*key) {
            output.insert((*key).to_string(), value.clone());
        }
    }
}

/// Convert persisted SQLite boolean integers into the render-ready JSON boolean contract.
fn insert_bool_alias(output: &mut Map<String, Value>, row: &Value, name: &str, keys: &[&str]) {
    let value = match pick(row, keys) {
        Value::Bool(value) => Some(value),
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        _ => None,
    };
    if let Some(value) = value {
        output.insert(name.to_string(), Value::Bool(value));
    }
}

/// Read a persisted capability without accepting stringly-typed authority values.
fn persisted_bool(row: &Value, keys: &[&str]) -> Option<bool> {
    let mut parsed = None;
    for key in keys {
        let Some(value) = row.get(*key) else {
            continue;
        };
        let Some(current) = (match value {
            Value::Bool(value) => Some(*value),
            Value::Number(value) => match value.as_i64() {
                Some(0) => Some(false),
                Some(1) => Some(true),
                _ => None,
            },
            _ => None,
        }) else {
            // An invalid alias is an authority conflict, never a reason to trust another alias.
            return None;
        };
        if parsed.is_some_and(|previous| previous != current) {
            return None;
        }
        parsed = Some(current);
    }
    parsed
}

/// Read a persisted role without accepting unknown role strings as authority.
fn persisted_role(row: &Value) -> Option<String> {
    row.get("role")
        .and_then(Value::as_str)
        .map(str::trim)
        .map(str::to_ascii_uppercase)
        .filter(|role| matches!(role.as_str(), "CREATOR" | "MANAGER" | "MEMBER" | "BOSS"))
}

/// Read a persisted permission threshold and fail closed for invalid values.
fn persisted_role_value(row: &Value, keys: &[&str]) -> Option<String> {
    let mut parsed = None;
    for key in keys {
        let Some(value) = row.get(*key).and_then(Value::as_str) else {
            if row.get(*key).is_some() {
                return None;
            }
            continue;
        };
        let normalized = value.trim().to_ascii_uppercase();
        if !matches!(
            normalized.as_str(),
            "CREATOR" | "MANAGER" | "MEMBER" | "BOSS"
        ) {
            return None;
        }
        if parsed
            .as_deref()
            .is_some_and(|previous| previous != normalized)
        {
            return None;
        }
        parsed = Some(normalized);
    }
    parsed
}

/// Compare role thresholds using the Go ordering: BOSS, CREATOR, MANAGER, MEMBER.
fn role_at_or_above(role: Option<&str>, required: Option<&str>) -> bool {
    let role_level = match role {
        Some("BOSS") => 0,
        Some("CREATOR") => 1,
        Some("MANAGER") => 2,
        Some("MEMBER") => 3,
        _ => return false,
    };
    let required_level = match required {
        Some("BOSS") => 0,
        Some("CREATOR") => 1,
        Some("MANAGER") => 2,
        Some("MEMBER") => 3,
        _ => return false,
    };
    role_level <= required_level
}

/// Project absolute settings/member capabilities, preserving persisted Go fields when present.
fn insert_viewer_capabilities(output: &mut Map<String, Value>, row: &Value) {
    let role = persisted_role(row);
    let is_open = matches!(
        pick(row, &["type"]),
        Value::String(ref value) if value.eq_ignore_ascii_case("O")
    );
    let admin_role = matches!(role.as_deref(), Some("CREATOR" | "MANAGER"))
        || (is_open && matches!(role.as_deref(), Some("BOSS")));
    let change_role = matches!(role.as_deref(), Some("CREATOR"))
        || (is_open && matches!(role.as_deref(), Some("BOSS")));
    let derived_settings = admin_role;
    let derived_members = admin_role;
    let derived_notice = role_at_or_above(
        role.as_deref(),
        persisted_role_value(row, &["notice_permission", "noticePermission"]).as_deref(),
    );
    let derived_top = role_at_or_above(
        role.as_deref(),
        persisted_role_value(row, &["top_permission", "topPermission"]).as_deref(),
    );
    let can_edit = persisted_bool(
        row,
        &["can_edit_channel_settings", "canEditChannelSettings"],
    )
    .unwrap_or(derived_settings);
    let can_manage_settings =
        persisted_bool(row, &["can_manage_settings", "canManageSettings"]).unwrap_or(can_edit);
    let can_manage_members =
        persisted_bool(row, &["can_manage_members", "canManageMembers"]).unwrap_or(derived_members);
    let can_publish_notice =
        persisted_bool(row, &["can_publish_notice", "canPublishNotice"]).unwrap_or(derived_notice);
    let can_change_permissions =
        persisted_bool(row, &["can_change_permissions", "canChangePermissions"])
            .unwrap_or(change_role);
    let can_pin_message =
        persisted_bool(row, &["can_pin_message", "canPinMessage"]).unwrap_or(derived_top);
    output.insert("canEditChannelSettings".to_string(), Value::Bool(can_edit));
    output.insert(
        "canManageSettings".to_string(),
        Value::Bool(can_manage_settings),
    );
    output.insert(
        "canManageMembers".to_string(),
        Value::Bool(can_manage_members),
    );
    output.insert(
        "canPublishNotice".to_string(),
        Value::Bool(can_publish_notice),
    );
    output.insert(
        "canChangePermissions".to_string(),
        Value::Bool(can_change_permissions),
    );
    output.insert("canPinMessage".to_string(), Value::Bool(can_pin_message));
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
        "channelId",
        &["channel_id", "channelId", "id"],
    );
    insert_alias(
        &mut output,
        row,
        "displayName",
        &["display_name", "displayName"],
    );
    insert_alias(&mut output, row, "type", &["type"]);
    insert_alias(&mut output, row, "teamId", &["team_id", "teamId"]);
    insert_alias(
        &mut output,
        row,
        "companyId",
        &["company_id", "companyId", "team_id", "teamId"],
    );
    // Go still exposes channel.update_at for legacy settingVersion readers and the
    // permissionRevision read-side ordering hint; permission writes no longer send it
    // as a CAS condition. Prefer explicit columns when a newer schema has them.
    insert_alias(
        &mut output,
        row,
        "settingVersion",
        &[
            "setting_version",
            "settingVersion",
            "updated_at",
            "updateAt",
        ],
    );
    insert_alias(
        &mut output,
        row,
        "permissionRevision",
        &[
            "permission_revision",
            "permissionRevision",
            "updated_at",
            "updateAt",
        ],
    );
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
    insert_non_negative_count_alias(
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
    insert_viewer_capabilities(&mut output, row);
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
    insert_bool_alias(
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
    insert_bool_alias(
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
                "has_urgent_post": 1,
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
        assert_eq!(shaped[0]["hasUrgentPost"], true);
        assert_eq!(shaped[0]["lastEventSeq"], 151);
        assert_eq!(shaped[0]["lastReadSeq"], 9);
        assert_eq!(shaped[0]["projectionRevision"], 12);
        assert_eq!(shaped[0]["channelIsTop"], true);
        assert_eq!(shaped[0]["picture"]["userIds"][0], "444");
        assert_eq!(shaped[0]["notifyProps"], "IGNORE");
        assert_eq!(shaped[0]["source"], "sync_events");
        assert_eq!(shaped[0]["lastPost"]["message"], "回复消息");
        assert_eq!(shaped[1]["displayName"], "空群");
        assert_eq!(shaped.as_array().map(Vec::len), Some(2));
    }

    /// Legacy negative urgent counters become zero without changing the urgent post list.
    #[test]
    fn clamps_negative_urgent_count_for_snake_and_camel_rows() {
        let rows = json!([
            {
                "id": "channel-negative-snake",
                "urgent_count": -2,
                "urgent_post_list": "[\"post-1\"]"
            },
            {
                "id": "channel-negative-camel",
                "urgentCount": -3,
                "urgentPostList": ["post-2"]
            }
        ]);

        let shaped = shape_channel_rows(&rows);
        assert_eq!(shaped[0]["urgentCount"], 0);
        assert_eq!(shaped[0]["urgent_count"], 0);
        assert_eq!(shaped[0]["urgentPostList"], json!(["post-1"]));
        assert_eq!(shaped[1]["urgentCount"], 0);
        assert_eq!(shaped[1]["urgentPostList"], json!(["post-2"]));
    }

    /// A single legacy negative row must not invalidate a twenty-row channel page.
    #[test]
    fn keeps_twenty_row_page_renderable_with_one_negative_urgent_count() {
        let rows = Value::Array(
            (0..20)
                .map(|index| {
                    json!({
                        "id": format!("channel-{index}"),
                        "urgent_count": if index == 7 { -1 } else { index },
                    })
                })
                .collect(),
        );

        let shaped = shape_channel_rows(&rows);
        assert_eq!(shaped.as_array().map(Vec::len), Some(20));
        assert_eq!(shaped[7]["urgentCount"], 0);
        assert_eq!(shaped[6]["urgentCount"], 6);
        assert_eq!(shaped[19]["urgentCount"], 19);
    }

    /// Channel pages expose the Go-equivalent absolute capabilities for every persisted viewer role.
    #[test]
    fn projects_viewer_capabilities_from_authoritative_role() {
        let rows = json!([
            { "id": "creator", "type": "P", "role": "CREATOR", "noticePermission": "MANAGER", "topPermission": "MANAGER" },
            { "id": "manager", "type": "P", "role": "MANAGER", "noticePermission": "MANAGER", "topPermission": "MANAGER" },
            { "id": "boss", "type": "O", "role": "BOSS", "noticePermission": "MANAGER", "topPermission": "MANAGER" },
            { "id": "member", "type": "P", "role": "MEMBER", "noticePermission": "MANAGER", "topPermission": "MANAGER" },
            { "id": "invalid", "type": "P", "role": "UNKNOWN", "noticePermission": "MANAGER", "topPermission": "MANAGER" },
            { "id": "missing", "type": "P" }
        ]);

        let shaped = shape_channel_rows(&rows);
        for id in ["creator", "manager", "boss"] {
            let row = shaped
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap();
            assert_eq!(row["canManageSettings"], true, "{id}");
            assert_eq!(row["canManageMembers"], true, "{id}");
            assert_eq!(row["canEditChannelSettings"], true, "{id}");
            assert_eq!(row["canChangePermissions"], id != "manager", "{id}");
            assert_eq!(row["canPublishNotice"], true, "{id}");
            assert_eq!(row["canPinMessage"], true, "{id}");
        }
        for id in ["member", "invalid", "missing"] {
            let row = shaped
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap();
            assert_eq!(row["canManageSettings"], false, "{id}");
            assert_eq!(row["canManageMembers"], false, "{id}");
            assert_eq!(row["canEditChannelSettings"], false, "{id}");
            assert_eq!(row["canPublishNotice"], false, "{id}");
            assert_eq!(row["canChangePermissions"], false, "{id}");
            assert_eq!(row["canPinMessage"], false, "{id}");
        }
    }

    /// Invalid policy values and stringly-typed booleans never grant a capability.
    #[test]
    fn fails_closed_for_invalid_policy_and_boolean_values() {
        let shaped = shape_channel_row(&json!({
            "id": "invalid-policy",
            "type": "O",
            "role": "BOSS",
            "noticePermission": "UNKNOWN",
            "topPermission": "UNKNOWN",
            "can_publish_notice": "true",
            "can_pin_message": 2
        }));
        assert_eq!(shaped["canPublishNotice"], false);
        assert_eq!(shaped["canPinMessage"], false);
        assert_eq!(shaped["canManageMembers"], true);
    }

    /// Persisted absolute fields win over role fallback and remain JSON booleans.
    #[test]
    fn preserves_persisted_viewer_capabilities_without_role_inference() {
        let shaped = shape_channel_row(&json!({
            "id": "channel-1",
            "role": "MEMBER",
            "can_manage_settings": 1,
            "canManageMembers": false,
        }));

        assert_eq!(shaped["canManageSettings"], true);
        assert_eq!(shaped["canManageMembers"], false);
    }

    /// 回归锚: legacy channel rows use the durable update timestamp as the Go CAS version.
    #[test]
    fn restores_cas_versions_from_legacy_updated_at_rows() {
        let shaped = shape_channel_row(&json!({
            "id": "channel-cas-version",
            "type": "P",
            "role": "CREATOR",
            "updated_at": 1_786_001_262_956_i64,
        }));

        assert_eq!(shaped["settingVersion"], 1_786_001_262_956_i64);
        assert_eq!(shaped["permissionRevision"], 1_786_001_262_956_i64);

        let explicit = shape_channel_row(&json!({
            "id": "channel-explicit-version",
            "updated_at": 10_i64,
            "setting_version": 11_i64,
            "permission_revision": 12_i64,
        }));

        assert_eq!(explicit["settingVersion"], 11_i64);
        assert_eq!(explicit["permissionRevision"], 12_i64);
    }
}
