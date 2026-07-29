//! 合并转发 durable props 到 MessageV3 `forwardDetail` 的收敛边界。

use serde_json::{json, Value};

const MAX_FORWARD_ITEMS: usize = 100;

/// 将 MULTIPLY props 收敛为 bounded detail，非法 authority 整体 fail closed。
pub(super) fn detail(props: &Value) -> Value {
    let Some(items) = decode_items(props) else {
        return unavailable("invalid_merge_message");
    };
    if items.len() > MAX_FORWARD_ITEMS {
        return unavailable("too_many_merge_items");
    }
    let Some(items) = normalize_items(items) else {
        return unavailable("invalid_merge_item");
    };
    let item_count = items.len();
    json!({
        "mode": "merged",
        "state": "ready",
        "title": string_at(props, &["forwardTitle", "forward_title"])
            .unwrap_or_else(|| "聊天记录".to_string()),
        "summary": format!("共{item_count}条消息"),
        "itemCount": item_count,
        "sourceChannelId": string_at(props, &["sourceChannelId", "source_channel_id"]),
        "sourceChannelName": string_at(props, &["sourceChannelName", "source_channel_name"]),
        "items": items,
        "failureReason": Value::Null,
    })
}

/// 解出 string/array 两种远端 mergeMessage authority。
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

/// 逐项收敛 snake/camel 别名，任一非法项使整个合并记录不可用。
fn normalize_items(items: Vec<Value>) -> Option<Vec<Value>> {
    items.into_iter().map(normalize_item).collect()
}

/// 将一个来源消息收敛为稳定 identity、作者、文本和时间。
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
    let user_id = string_at(&item, &["userId", "user_id"])
        .or_else(|| string_at(&snapshot, &["userId", "user_id"]))
        .unwrap_or_default();
    let msg_type = string_at(&item, &["type"]).unwrap_or_else(|| "TEXT".to_string());
    let create_at = item
        .get("createAt")
        .or_else(|| item.get("create_at"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    Some(json!({
        "id": id,
        "userId": user_id,
        "author": author.unwrap_or_default(),
        "text": text,
        "type": msg_type,
        "createAt": create_at,
        "userSnapshot": snapshot,
    }))
}

/// 按优先级读取首个非空字符串别名。
fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 构造不可解析合并记录的稳定终态。
fn unavailable(reason: &'static str) -> Value {
    json!({
        "mode": "merged",
        "state": "unavailable",
        "title": "聊天记录",
        "summary": "聊天记录暂不可用",
        "itemCount": 0,
        "sourceChannelId": Value::Null,
        "sourceChannelName": Value::Null,
        "items": [],
        "failureReason": reason,
    })
}
