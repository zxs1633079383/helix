//! Merged-forward props -> render-ready detail.

use serde_json::{json, Value};

pub(crate) fn detail(msg_type: &str, props: &Value) -> Value {
    if !msg_type.eq_ignore_ascii_case("MULTIPLY") {
        return Value::Null;
    }

    let Some(items) = decode_items(props) else {
        return unavailable("invalid_merge_message");
    };
    let Some(items) = normalize_items(items) else {
        return unavailable("invalid_merge_item");
    };
    let item_count = items.len();
    json!({
        "mode": "merged",
        "state": "ready",
        "title": "聊天记录",
        "summary": format!("共{item_count}条消息"),
        "itemCount": item_count,
        "sourceChannelName": string_at(props, &["sourceChannelName", "source_channel_name"]),
        "items": items,
        "failureReason": Value::Null,
    })
}

fn decode_items(props: &Value) -> Option<Vec<Value>> {
    let raw = props
        .get("mergeMessage")
        .or_else(|| props.get("merge_message"))?;
    match raw {
        Value::Array(items) => Some(items.clone()),
        Value::String(encoded) => serde_json::from_str::<Value>(encoded)
            .ok()?
            .as_array()
            .cloned(),
        _ => None,
    }
}

fn normalize_items(items: Vec<Value>) -> Option<Vec<Value>> {
    items.into_iter().map(normalize_item).collect()
}

fn normalize_item(item: Value) -> Option<Value> {
    let Value::Object(object) = item else {
        return None;
    };
    let item = Value::Object(object);
    let snapshot = item
        .get("userSnapshot")
        .or_else(|| item.get("user_snapshot"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let author = string_at(&item, &["author", "userName", "user_name"])
        .or_else(|| string_at(&snapshot, &["displayName", "userName", "user_name", "name"]));
    let text = string_at(&item, &["text", "message", "content"]).unwrap_or_default();
    let id = string_at(&item, &["id", "temporaryId", "temporary_id"]).unwrap_or_default();
    let msg_type = string_at(&item, &["type"]).unwrap_or_else(|| "TEXT".to_string());
    let create_at = item
        .get("createAt")
        .or_else(|| item.get("create_at"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    Some(json!({
        "id": id,
        "author": author.unwrap_or_default(),
        "text": text,
        "type": msg_type,
        "createAt": create_at,
        "userSnapshot": snapshot,
    }))
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn unavailable(reason: &'static str) -> Value {
    json!({
        "mode": "merged",
        "state": "unavailable",
        "title": "聊天记录",
        "summary": "聊天记录暂不可用",
        "itemCount": 0,
        "sourceChannelName": Value::Null,
        "items": [],
        "failureReason": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_string_and_array_shapes_to_the_same_detail() {
        let items = json!([{
            "temporaryId":"p1",
            "message":"你好",
            "type":"TEXT",
            "createAt":7,
            "userSnapshot":{"userName":"甲"}
        }]);
        let string_props = json!({"mergeMessage": items.to_string()});
        let array_props = json!({"mergeMessage": items});
        let from_string = detail("MULTIPLY", &string_props);
        let from_array = detail("MULTIPLY", &array_props);
        assert_eq!(from_string, from_array);
        assert_eq!(from_string["state"], "ready");
        assert_eq!(from_string["items"][0]["author"], "甲");
        assert_eq!(from_string["items"][0]["text"], "你好");
    }

    #[test]
    fn malformed_merged_payload_fails_closed() {
        let result = detail("MULTIPLY", &json!({"mergeMessage":"not-json"}));
        assert_eq!(result["state"], "unavailable");
        assert_eq!(result["items"], json!([]));
        assert_eq!(result["failureReason"], "invalid_merge_message");
    }
}
