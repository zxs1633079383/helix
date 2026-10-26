use serde_json::{json, Value};

use crate::module::ImModule;
use crate::outbound::send_build::{build_message_object, BuildInput, UserIdentity};
use crate::state::{ChannelId, TemporaryId};
use crate::ImError;

pub(super) fn decode_single_row(bytes: &[u8]) -> Result<Option<Value>, ImError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    let rows: Vec<Value> = serde_json::from_slice(bytes)
        .map_err(|error| ImError::Parse(format!("im_retry_send durable row: {error}")))?;
    Ok(rows.into_iter().next().filter(Value::is_object))
}

pub(super) fn rebuild_body(
    module: &ImModule,
    row: &Value,
    temporary_id: &TemporaryId,
    requested_at_ms: u64,
) -> Result<Value, ImError> {
    let channel_id = row_str(row, "channel_id");
    ChannelId::from_str(channel_id).ok_or_else(|| {
        ImError::Parse("im_retry_send durable message missing channel_id".to_string())
    })?;
    let msg_type = match row_str(row, "type") {
        "" => "TEXT",
        value => value,
    };
    let props = row_json(row, "props", json!({}));
    let viewers = row_json(row, "viewers", json!([]))
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let mentions = row_json(row, "mentions", json!([]));
    let create_at = row
        .get("create_at")
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .unwrap_or(requested_at_ms);
    let identity = module.config.user_identity();
    let mut body = build_message_object(&BuildInput {
        channel_id,
        temporary_id: temporary_id.0.as_str(),
        msg_type,
        post_id: row_str(row, "id"),
        text: row_str(row, "message"),
        viewers,
        mentions,
        props,
        topic_id: row_str(row, "topic"),
        replied: None,
        now_ms: create_at,
        identity: UserIdentity {
            user_id: identity.user_id,
            team_id: identity.team_id,
            user_name: identity.user_name,
            org_name: identity.org_name,
            dept_name: identity.dept_name,
        },
    })
    .ok_or_else(|| ImError::Parse("im_retry_send durable message is empty".to_string()))?;

    for (column, wire_key) in [
        ("reply_id", "replyId"),
        ("reply_root_id", "replyRootId"),
        ("reply_first_level_id", "replyFirstLevelId"),
    ] {
        let value = row_str(row, column);
        if !value.is_empty() {
            body[wire_key] = Value::String(value.to_string());
        }
    }
    let snapshot = row_json(row, "user_snapshot", json!({}));
    if snapshot.as_object().is_some_and(|value| !value.is_empty()) {
        if let Some(team_id) = snapshot.get("teamId").and_then(Value::as_str) {
            body["teamId"] = Value::String(team_id.to_string());
        }
    }
    let team_id = row_str(row, "team_id");
    if !team_id.is_empty() {
        body["teamId"] = Value::String(team_id.to_string());
    }
    let user_id = row_str(row, "user_id");
    if !user_id.is_empty() {
        body["userId"] = Value::String(user_id.to_string());
    }
    let replied = row_json(row, "replied_message", Value::Null);
    if replied.is_object() {
        body["repliedMessage"] = crate::message_identity::sanitize(replied);
    }
    let simple_message = row_str(row, "simple_message");
    if !simple_message.is_empty() {
        body["simpleMessage"] = Value::String(simple_message.to_string());
    }
    Ok(crate::message_identity::sanitize(body))
}

pub(super) fn row_str<'a>(row: &'a Value, key: &str) -> &'a str {
    row.get(key).and_then(Value::as_str).unwrap_or("")
}

fn row_json(row: &Value, key: &str, fallback: Value) -> Value {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|raw| !raw.is_empty())
        .and_then(|raw| serde_json::from_str(raw).ok())
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImConfig;

    /// Retry retains company and reply content without restoring obsolete login/profile snapshots.
    #[test]
    fn retry_uses_ids_and_keeps_reply_content_without_names() {
        let module = ImModule::new(ImConfig::default());
        let row = json!({"channel_id":"a9h5hrdsy3873dmg375a6ntqiw","type":"TEXT","message":"reply",
            "user_id":"sender","team_id":"source-company","user_snapshot":"{\"userName\":\"wrong\",\"teamId\":\"old-company\"}",
            "replied_message":"{\"repliedUserId\":\"original\",\"teamId\":\"reply-company\",\"repliedUserName\":\"wrong\",\"message\":\"original content\"}"});
        let body = rebuild_body(&module, &row, &TemporaryId("temporary-1".into()), 10).unwrap();
        assert_eq!(body["userId"], "sender");
        assert_eq!(body["teamId"], "source-company");
        assert!(body.get("userSnapshot").is_none());
        assert_eq!(body["repliedMessage"]["message"], "original content");
        assert_eq!(body["repliedMessage"]["repliedTeamId"], "reply-company");
        assert!(body["repliedMessage"].get("repliedUserName").is_none());
    }
}
