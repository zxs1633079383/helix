//! Message identity is an ID reference; display profiles belong to the host user directory.
use serde_json::Value;

/// Remove only declared identity snapshots, retaining IDs and all message/business content.
pub(crate) fn sanitize(mut value: Value) -> Value {
    match &mut value {
        Value::Array(items) => {
            for item in items {
                *item = sanitize(item.take());
            }
        }
        Value::Object(object) => {
            for (snapshot_key, id_key) in [
                ("userSnapshot", "userId"),
                ("user_snapshot", "userId"),
                ("authorSnapshot", "authorId"),
                ("author_snapshot", "authorId"),
                ("creatorSnapshot", "creatorId"),
                ("creator_snapshot", "creatorId"),
            ] {
                if let Some(mut snapshot) = object.remove(snapshot_key) {
                    if let Some(raw) = snapshot.as_str() {
                        snapshot = serde_json::from_str(raw).unwrap_or(Value::Null);
                    }
                    for (target, aliases) in [
                        (id_key, ["userId", "user_id", "id"]),
                        ("teamId", ["teamId", "companyId", "team_id"]),
                    ] {
                        let snake = match target {
                            "userId" => "user_id",
                            "authorId" => "author_id",
                            "creatorId" => "creator_id",
                            _ => "team_id",
                        };
                        if object
                            .get(target)
                            .or_else(|| object.get(snake))
                            .and_then(Value::as_str)
                            .is_none_or(str::is_empty)
                        {
                            if let Some(id) = aliases.iter().find_map(|key| {
                                snapshot
                                    .get(*key)
                                    .and_then(Value::as_str)
                                    .filter(|id| !id.is_empty())
                            }) {
                                object.insert(target.to_owned(), Value::String(id.to_owned()));
                            }
                        }
                    }
                }
            }
            for (snake, camel) in [
                ("reply_user_id", "replyUserId"),
                ("reply_team_id", "replyTeamId"),
                ("replied_user_id", "repliedUserId"),
                ("replied_team_id", "repliedTeamId"),
            ] {
                if let Some(id) = object.remove(snake) {
                    object.entry(camel.to_owned()).or_insert(id);
                }
            }
            if object.contains_key("repliedUserId") && !object.contains_key("replyUserId") {
                if let Some(team) = object.remove("teamId").or_else(|| object.remove("team_id")) {
                    object.entry("repliedTeamId".to_owned()).or_insert(team);
                }
            }
            object.remove("replyUserName");
            object.remove("reply_user_name");
            object.remove("repliedUserName");
            object.remove("replied_user_name");
            // Forward preview author was a frozen display name, never message content.
            if object.contains_key("snapshotId") && object.contains_key("preview") {
                if let Some(items) = object.get_mut("preview").and_then(Value::as_array_mut) {
                    for item in items {
                        if let Some(item) = item.as_object_mut() {
                            item.remove("author");
                        }
                    }
                }
            }
            for (key, child) in object.iter_mut() {
                // Legacy merged forwards encode message trees as JSON text; plain message text is content.
                if matches!(key.as_str(), "mergeMessage" | "merge_message") {
                    if let Some(raw) = child.as_str() {
                        if let Ok(encoded) = serde_json::from_str::<Value>(raw) {
                            if encoded.is_object() || encoded.is_array() {
                                *child = Value::String(sanitize(encoded).to_string());
                                continue;
                            }
                        }
                    }
                }
                *child = sanitize(child.take());
            }
        }
        _ => {}
    }
    value
}

/// Sanitize an owned JSON column without altering non-JSON content.
pub(crate) fn json_text(raw: &str) -> String {
    serde_json::from_str(raw)
        .map(sanitize)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Old nested reply/forward identities cannot repopulate profiles; business text survives.
    #[test]
    fn legacy_profiles_keep_ids_only() {
        let clean = sanitize(
            json!({"userSnapshot":{"userId":"u1","teamId":"c1","userName":"wrong"},
            "repliedMessage":{"repliedUserId":"u2","repliedUserName":"wrong","message":"original"},
            "replyMessages":{"p1":{"replyUserId":"u3","reply_team_id":"c3","replyUserName":"wrong","replied_user_id":"u4","replied_team_id":"c4","message":"reply"}},
            "props":{"task":{"name":"task name"},"forward":{"snapshotId":"s1","preview":[{"author":"wrong","userId":"u2","teamId":"c2","text":"content"}]}}}),
        );
        assert_eq!(clean["userId"], "u1");
        assert_eq!(clean["teamId"], "c1");
        assert!(clean.get("userSnapshot").is_none());
        assert!(clean["repliedMessage"].get("repliedUserName").is_none());
        assert_eq!(clean["repliedMessage"]["message"], "original");
        assert_eq!(clean["props"]["task"]["name"], "task name");
        assert!(clean["props"]["forward"]["preview"][0]
            .get("author")
            .is_none());
        assert_eq!(clean["replyMessages"]["p1"]["replyTeamId"], "c3");
        assert_eq!(clean["replyMessages"]["p1"]["repliedTeamId"], "c4");
        assert!(clean["replyMessages"]["p1"].get("replyUserName").is_none());
        assert_eq!(sanitize(clean.clone()), clean);
    }
    /// Legacy JSON-text containers are cleaned recursively without changing their encoding or body text.
    #[test]
    fn encoded_merge_message_removes_profiles_but_preserves_text() {
        let body = r#"{"userSnapshot":{"userName":"正文不是身份"}}"#;
        let inner = json!([{ "userSnapshot":{"userId":"u2","teamId":"c2","userName":"旧名"},"message":body }]);
        let outer = json!([{ "userSnapshot":{"userId":"u1","teamId":"c1","userName":"旧名"},
            "message":"原始正文", "props":{"mergeMessage":inner.to_string()} }]);
        let clean = sanitize(json!({"props":{"mergeMessage":outer.to_string()},"message":body}));
        let outer: Value =
            serde_json::from_str(clean["props"]["mergeMessage"].as_str().unwrap()).unwrap();
        let inner: Value =
            serde_json::from_str(outer[0]["props"]["mergeMessage"].as_str().unwrap()).unwrap();
        assert_eq!(outer[0]["userId"], "u1");
        assert_eq!(outer[0]["teamId"], "c1");
        assert!(outer[0].get("userSnapshot").is_none());
        assert_eq!(outer[0]["message"], "原始正文");
        assert_eq!(inner[0]["userId"], "u2");
        assert_eq!(inner[0]["teamId"], "c2");
        assert!(inner[0].get("userSnapshot").is_none());
        assert_eq!(inner[0]["message"], body);
        assert_eq!(clean["message"], body);
        assert_eq!(sanitize(clean.clone()), clean);
    }
}
