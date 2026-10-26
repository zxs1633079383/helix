//! Immutable merged-forward cards and one-page snapshot projection; never decode descendants.

use crate::ImError;
use serde_json::{json, Value};

/// Validate the server-owned bounded card and copy only its declared fields.
pub(crate) fn card(value: &Value) -> Result<Value, ImError> {
    let snapshot_id = text(value, "snapshotId", 128)?;
    if snapshot_id.is_empty() {
        return Err(invalid("empty snapshotId"));
    }
    let title = text(value, "title", 128)?;
    let count = value["itemCount"]
        .as_u64()
        .filter(|n| (1..=100).contains(n))
        .ok_or_else(|| invalid("card itemCount"))?;
    let preview = value["preview"]
        .as_array()
        .filter(|items| items.len() <= 3)
        .ok_or_else(|| invalid("card preview"))?;
    let preview = preview
        .iter()
        .map(|item| {
            Ok(json!({
                "userId": optional_identity(item, "userId")?, "teamId": optional_identity(item, "teamId")?,
                "text": text(item, "text", 256)?
            }))
        })
        .collect::<Result<Vec<_>, ImError>>()?;
    if value.as_object().is_none_or(|object| object.len() != 4) {
        return Err(invalid("undeclared card field"));
    }
    Ok(json!({"snapshotId":snapshot_id,"title":title,"itemCount":count,"preview":preview}))
}

/// MULTIPLY never exports source trees, even if a corrupt local row contains extra props.
pub(crate) fn props(msg_type: &str, value: Value) -> Value {
    if !msg_type.eq_ignore_ascii_case("MULTIPLY") {
        return value;
    }
    card(&value["forward"])
        .map(|card| json!({"forward":card}))
        .unwrap_or_else(|_| json!({}))
}

/// Turn a bounded current-layer item into the same ChatMessage shape as history.
pub(crate) fn item(snapshot_id: &str, item: &Value, viewer: &str) -> Result<Value, ImError> {
    let clean_item = crate::message_identity::sanitize(item.clone());
    let item = &clean_item;
    let item_key = text(item, "itemKey", 128)?;
    if item_key.is_empty() {
        return Err(invalid("empty itemKey"));
    }
    if text(item, "userId", 128)?.is_empty() {
        return Err(invalid("empty userId"));
    }
    if !item["createAt"].is_i64() {
        return Err(invalid("item author/time"));
    }
    let id = format!("forward:{snapshot_id}:{item_key}");
    let mut row = json!({"id":id,"temporaryId":id,"userId":item["userId"],
        "teamId":item["teamId"],"createAt":item["createAt"]});
    let fields: &[&str] = match item["kind"].as_str() {
        Some("message") => &[
            "kind",
            "itemKey",
            "userId",
            "teamId",
            "companyId",
            "createAt",
            "type",
            "message",
            "simpleMessage",
            "props",
            "channelId",
            "viewers",
        ],
        Some("forward") => &[
            "kind",
            "itemKey",
            "userId",
            "teamId",
            "companyId",
            "createAt",
            "snapshotId",
            "title",
            "itemCount",
            "preview",
        ],
        _ => return Err(invalid("item kind")),
    };
    if item
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !fields.contains(&key.as_str())))
    {
        return Err(invalid("undeclared item field"));
    }
    match item["kind"].as_str() {
        Some("message") => {
            let kind = text(item, "type", 64)?;
            if kind.is_empty()
                || kind.eq_ignore_ascii_case("MULTIPLY")
                || !item["message"].is_string()
                || !item["props"].is_object()
            {
                return Err(invalid("message item"));
            }
            for key in ["mergeMessage", "merge_message", "forwardDetail", "forward"] {
                if item["props"].get(key).is_some() {
                    return Err(invalid("recursive message props"));
                }
            }
            // A forwarded business card still addresses the original business post.
            // Only the render key belongs to the immutable snapshot; mutations must
            // never send that synthetic key as a post ID.
            if matches!(
                kind,
                "TASK_CARD"
                    | "TASK_CARD_BLOCK"
                    | "MEETING_CARD_BLOCK"
                    | "DOCUMENT"
                    | "CHAIN"
                    | "TEXT_CHAIN"
                    | "CATEGORY_CHAIN"
                    | "VOTE"
                    | "AVERAGE_SCORE"
                    | "ANNOUNCEMENT"
            ) || kind == "TEXT" && item["props"].get("meetingCardBlock").is_some()
            {
                row["id"] = json!(item_key);
                let channel_id = text(item, "channelId", 128)?;
                if channel_id.is_empty() {
                    return Err(invalid("business channelId"));
                }
                row["channelId"] = json!(channel_id);
                let viewers = item["viewers"]
                    .as_array()
                    .ok_or_else(|| invalid("business viewers"))?;
                if viewers
                    .iter()
                    .any(|value| value.as_str().is_none_or(|id| id.len() > 128))
                {
                    return Err(invalid("business viewer"));
                }
                row["viewers"] = item["viewers"].clone();
            } else if item.get("channelId").is_some() || item.get("viewers").is_some() {
                return Err(invalid("non-business source context"));
            }
            row["type"] = item["type"].clone();
            row["message"] = item["message"].clone();
            row["props"] = item["props"].clone();
            if item.get("simpleMessage").is_some() {
                row["simpleMessage"] = json!(text(item, "simpleMessage", 256)?);
            }
            crate::event::post::canonicalize_media_file_names(&mut row["props"]);
        }
        Some("forward") => {
            let card = card(
                &json!({"snapshotId":item["snapshotId"],"title":item["title"],"itemCount":item["itemCount"],"preview":item["preview"]}),
            )?;
            row["type"] = json!("MULTIPLY");
            row["message"] = json!("");
            row["simpleMessage"] = json!("[聊天记录]");
            row["props"] = json!({"forward":card});
        }
        _ => return Err(invalid("item kind")),
    }
    let mut projected = super::core::shape_row(&row, viewer);
    projected["forwardItemKey"] = json!(item_key);
    Ok(projected)
}

pub(crate) fn text<'a>(value: &'a Value, key: &str, max: usize) -> Result<&'a str, ImError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| s.len() <= max)
        .ok_or_else(|| invalid(key))
}

/// Legacy preview cards lacked IDs; never reuse their frozen author name.
fn optional_identity<'a>(value: &'a Value, key: &str) -> Result<&'a str, ImError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(_) => text(value, key, 128),
    }
}

fn invalid(field: &str) -> ImError {
    ImError::Parse(format!("invalid forward {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn business_cards_keep_source_identity_and_separate_render_identity() {
        for kind in [
            "TASK_CARD",
            "TASK_CARD_BLOCK",
            "MEETING_CARD_BLOCK",
            "DOCUMENT",
            "CHAIN",
            "TEXT_CHAIN",
            "CATEGORY_CHAIN",
            "VOTE",
            "AVERAGE_SCORE",
            "ANNOUNCEMENT",
        ] {
            let source = json!({"kind":"message","itemKey":"source-post","userId":"author",
                "userSnapshot":{},"createAt":1,"type":kind,"message":"content",
                "props":{"businessId":"original-object"},"channelId":"source-channel","viewers":["viewer"]});
            let a = item("snapshot-a", &source, "viewer").unwrap();
            let b = item("snapshot-b", &source, "viewer").unwrap();
            assert_eq!(a["id"], "source-post");
            assert_eq!(a["channelId"], "source-channel");
            assert_eq!(a["viewers"], source["viewers"]);
            assert_eq!(a["props"], source["props"]);
            assert_ne!(a["temporaryId"], b["temporaryId"]);
            let mut invalid = source.clone();
            invalid["channelId"] = json!("");
            assert!(item("s", &invalid, "viewer").is_err());
            invalid = source.clone();
            invalid["viewers"] = json!([1]);
            assert!(item("s", &invalid, "viewer").is_err());
            invalid = source.clone();
            invalid["type"] = json!("TEXT");
            assert!(item("s", &invalid, "viewer").is_err());
        }
    }

    #[test]
    fn current_task_and_meeting_wire_shapes_keep_business_context() {
        for (kind, props) in [
            (
                "TASK_CARD",
                json!({"taskCard":{"id":"task"},"taskCardBlock":{"taskId":"task"}}),
            ),
            ("TEXT", json!({"meetingCardBlock":{"meetingId":"meeting"}})),
        ] {
            let source = json!({"kind":"message","itemKey":"original-post","userId":"author",
                "userSnapshot":{},"createAt":1,"type":kind,"message":"card","props":props,
                "channelId":"original-channel","viewers":["all"]});
            let projected = item("snapshot", &source, "viewer").unwrap();
            assert_eq!(projected["id"], "original-post");
            assert_eq!(projected["type"], kind);
            assert_eq!(projected["props"], props);
            assert_eq!(projected["channelId"], "original-channel");
            assert_ne!(projected["temporaryId"], projected["id"]);
        }
    }

    /// Live authority and history export the same bounded card and no legacy tree.
    #[test]
    fn live_and_history_share_card_only() {
        let card = json!({"snapshotId":"s","title":"记录","itemCount":100,"preview":[]});
        let props = json!({"forward":card,"mergeMessage":"deliberately-unparsed"});
        let history = super::super::core::shape_row(&json!({"type":"MULTIPLY","props":props}), "");
        let event = crate::sync_session::EventEnvelope::new(
            crate::state::test_channel_id(1),
            crate::state::Seq(1),
            crate::sync_session::EventKind::PostUpsert,
            crate::sync_session::PostFields {
                msg_type: "MULTIPLY".into(),
                props: props.to_string(),
                ..Default::default()
            },
        );
        let live = crate::event::post::authority_projection(&event).unwrap();
        assert_eq!(history["props"], json!({"forward":card}));
        assert_eq!(history["props"], live.received_data["props"]);
        assert_eq!(history["props"], live.last_post["props"]);
        assert!(history.get("forwardDetail").is_none());
        assert!(live.received_data.get("forwardDetail").is_none());
        let last_post =
            json!({"type":"MULTIPLY","props":props,"forwardDetail":{"items":["never export"]}});
        for value in [last_post.clone(), json!(last_post.to_string())] {
            let channel = crate::event::channel::update(json!({"lastPost":value})).unwrap();
            let event: Value = serde_json::from_slice(&channel.into_bytes()).unwrap();
            assert_eq!(event["data"]["lastPost"]["props"], json!({"forward":card}));
            assert!(event["data"]["lastPost"].get("forwardDetail").is_none());
        }
    }
}
