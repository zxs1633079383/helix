//! Merged-forward props -> render-ready detail.

use serde_json::{json, Value};

const MAX_FORWARD_ITEMS: usize = 100;
const MAX_FORWARD_DEPTH: usize = 8;
const MAX_FORWARD_TOTAL_ITEMS: usize = 1000;

/// 将 durable MULTIPLY props 转成 bounded render-ready detail，非法记录整体 fail closed。
pub(crate) fn detail(msg_type: &str, props: &Value) -> Value {
    if !msg_type.eq_ignore_ascii_case("MULTIPLY") {
        return Value::Null;
    }

    let mut remaining = MAX_FORWARD_TOTAL_ITEMS;
    detail_at_depth(props, 0, &mut remaining)
}

/// 共享总条数与深度预算，防止嵌套记录指数展开；坏的子卡片不吞掉外层普通消息。
fn detail_at_depth(props: &Value, depth: usize, remaining: &mut usize) -> Value {
    if depth >= MAX_FORWARD_DEPTH {
        return unavailable("merge_depth_limit");
    }
    let Some(items) = decode_items(props) else {
        return unavailable("invalid_merge_message");
    };
    if items.len() > MAX_FORWARD_ITEMS {
        return unavailable("too_many_merge_items");
    }
    if items.len() > *remaining {
        return unavailable("merge_total_limit");
    }
    *remaining -= items.len();
    let Some(items) = items
        .into_iter()
        .map(|item| normalize_item(item, depth, remaining))
        .collect::<Option<Vec<_>>>()
    else {
        return unavailable("invalid_merge_item");
    };
    let item_count = items.len();
    json!({
        "mode": "merged",
        "state": "ready",
        "title": string_at(props, &["forwardTitle", "forward_title"]).unwrap_or_else(|| "聊天记录".to_string()),
        "summary": format!("共{item_count}条消息"),
        "itemCount": item_count,
        "sourceChannelId": string_at(props, &["sourceChannelId", "source_channel_id"]),
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

/// 单条来源记录统一 snake/camel 别名，并保留 strict item 所需的 id 与 userId。
fn normalize_item(item: Value, depth: usize, remaining: &mut usize) -> Option<Value> {
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
    let mut result = json!({
        "id": id,
        "userId": user_id,
        "author": author.unwrap_or_default(),
        "text": text,
        "type": msg_type,
        "createAt": create_at,
        "userSnapshot": snapshot,
    });
    let props = item.get("props").map(|value| match value {
        Value::String(raw) => serde_json::from_str(raw).unwrap_or(Value::Null),
        value => value.clone(),
    }).unwrap_or(Value::Null);
    result["simpleMessage"] = Value::String(crate::message_summary::resolve(
        &msg_type, &text, &props, &string_at(&item, &["simpleMessage", "simple_message"]).unwrap_or_default(), false,
    ));
    if msg_type == "MULTIPLY" {
        let props = match item.get("props") {
            Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(Value::Null),
            Some(value) => value.clone(),
            None => Value::Null,
        };
        result["forwardDetail"] = detail_at_depth(&props, depth + 1, remaining);
    }
    // 媒体详情沿用普通消息的 props；不把原始嵌套 mergeMessage 扩散到投影边界。
    if matches!(
        msg_type.as_str(),
        "RICH" | "IMAGE" | "VIDEO" | "FILE" | "AUDIO" | "VOICE"
    ) {
        let mut props = match item.get("props") {
            Some(Value::String(raw)) if !raw.trim().is_empty() => serde_json::from_str(raw).ok()?,
            Some(Value::Object(object)) => Value::Object(object.clone()),
            None | Some(Value::Null) => json!({}),
            Some(Value::String(_)) => json!({}),
            _ => return None,
        };
        let object = props.as_object_mut()?;
        object.remove("channel_event_seq");
        object.remove("mergeMessage");
        object.remove("merge_message");
        crate::event::post::canonicalize_media_file_names(&mut props);
        result["props"] = props;
    }
    Some(result)
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
        "sourceChannelId": Value::Null,
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

    /// 回归锚：6aa1289d / 6aa1289e；摘要与正文分离，嵌套子卡片仍可继续打开。
    #[test]
    fn preserves_summary_and_nested_detail_without_raw_merge_props() {
        let inner = json!({"mergeMessage": [{"id":"leaf", "type":"TEXT", "message":"完整正文", "simpleMessage":"摘要"}]});
        let props = json!({"mergeMessage": [
            {"id":"nested", "type":"MULTIPLY", "simple_message":"[聊天记录]", "props":inner.to_string()},
            {"id":"plain", "type":"TEXT", "message":"混排"}
        ]});
        let result = detail("MULTIPLY", &props);
        let nested = &result["items"][0];
        assert_eq!(nested["simpleMessage"], "[聊天记录]");
        assert!(nested.get("props").is_none());
        assert_eq!(nested["forwardDetail"]["items"][0]["text"], "完整正文");
        assert_eq!(nested["forwardDetail"]["items"][0]["simpleMessage"], "摘要");
        assert_eq!(result["items"][1]["text"], "混排");
        let history = super::super::core::shape_row(
            &json!({"type":"MULTIPLY", "props":props.to_string()}),
            "",
        );
        assert_eq!(history["forwardDetail"], result);
    }

    /// 超深/超总数和损坏子项局部不可用，有限外层仍可渲染。
    #[test]
    fn nested_projection_has_depth_and_total_bounds() {
        let mut props = json!({"mergeMessage": [{"type":"TEXT", "message":"leaf"}]});
        for _ in 0..MAX_FORWARD_DEPTH {
            props = json!({"mergeMessage": [{"type":"MULTIPLY", "props":props}]});
        }
        let result = detail("MULTIPLY", &props);
        let mut nested = &result;
        for _ in 0..MAX_FORWARD_DEPTH {
            nested = &nested["items"][0]["forwardDetail"];
        }
        assert_eq!(nested["failureReason"], "merge_depth_limit");
        let mut remaining = 0;
        assert_eq!(
            detail_at_depth(&props, 0, &mut remaining)["failureReason"],
            "merge_total_limit"
        );
        let broken = detail(
            "MULTIPLY",
            &json!({"mergeMessage":[{"type":"MULTIPLY","props":"broken"},{"type":"TEXT","message":"ok"}]}),
        );
        assert_eq!(broken["state"], "ready");
        assert_eq!(broken["items"][0]["forwardDetail"]["state"], "unavailable");
        assert_eq!(broken["items"][1]["text"], "ok");
    }

    #[test]
    fn malformed_merged_payload_fails_closed() {
        let result = detail("MULTIPLY", &json!({"mergeMessage":"not-json"}));
        assert_eq!(result["state"], "unavailable");
        assert_eq!(result["items"], json!([]));
        assert_eq!(result["failureReason"], "invalid_merge_message");
    }

    /// 历史查询与实时 authority 使用同一媒体归一化，保留图文顺序和视频属性。
    #[test]
    fn media_forward_detail_preserves_props_in_both_projection_paths() {
        use crate::state::{ChannelId, Seq};
        use crate::sync_session::{EventEnvelope, EventKind, PostFields};

        for kind in ["RICH", "IMAGE", "VIDEO", "FILE", "AUDIO", "VOICE"] {
            let file_name = if kind == "RICH" { "a.png" } else { "a.mp4" };
            let content_type = if kind == "RICH" { "image/png" } else { "video/mp4" };
            let media = json!({"url":"https://media.test/a", "fileName":file_name, "contentType":content_type, "width":640, "height":480});
            let props = if matches!(kind, "RICH" | "IMAGE") {
                let files = if kind == "RICH" { vec![media.clone(); 3] } else { vec![media] };
                json!({"files":files, "imagePosition":"bottom"})
            } else {
                json!({"file":media})
            };
            for encoded in [props.clone(), Value::String(props.to_string())] {
                let summary = if kind == "RICH" { "[图片] 3个 11 1 1 11" } else if kind == "VOICE" { "[语音]" } else { "已保存的媒体摘要" };
                let merged = json!({"mergeMessage":[
                    {"id":"first", "type":"TEXT", "message":"1", "simpleMessage":"1"},
                    {"id":"second", "type":"TEXT", "message":"1", "simpleMessage":"1"},
                    {"id":"m1", "type":kind, "message":"11\n1\n1\n11", "simpleMessage":summary, "props":encoded}
                ]});
                let history = super::super::core::shape_row(
                    &json!({"type":"MULTIPLY", "props":merged.to_string()}),
                    "",
                );
                let event = EventEnvelope::new(
                    ChannelId::from_str("a9h5hrdsy3873dmg375a6ntqiw").unwrap(),
                    Seq(8),
                    EventKind::PostUpsert,
                    PostFields {
                        msg_type: "MULTIPLY".to_string(),
                        props: merged.to_string(),
                        ..PostFields::default()
                    },
                );
                let live = crate::event::post::authority_projection(&event).unwrap();
                let detail = &live.received_data["forwardDetail"];
                assert_eq!(detail, &history["forwardDetail"]);
                assert_eq!(detail, &live.last_post["forwardDetail"]);
                assert_eq!(detail["items"][0]["simpleMessage"], "1");
                assert_eq!(detail["items"][1]["simpleMessage"], "1");
                let item = &detail["items"][2];
                assert_eq!(item["text"], "11\n1\n1\n11");
                assert_eq!(item["simpleMessage"], summary);
                let file = if matches!(kind, "RICH" | "IMAGE") {
                    assert_eq!(item["props"]["imagePosition"], "bottom");
                    &item["props"]["files"][0]
                } else {
                    &item["props"]["file"]
                };
                assert_eq!(file["url"], "https://media.test/a");
                assert_eq!(file["name"], file_name);
                assert_eq!(file["width"], 640);
                assert!(file.get("fileName").is_none());
            }
        }
    }

    /// 非法媒体 props 与过量条目不能变成看似成功的详情。
    #[test]
    fn media_forward_detail_rejects_invalid_props_and_overflow() {
        for props in [json!("bad-json"), json!("[]"), json!([]), json!(42)] {
            let result = detail(
                "MULTIPLY",
                &json!({"mergeMessage":[{"type":"RICH", "props":props}]}),
            );
            assert_eq!(result["failureReason"], "invalid_merge_item");
        }
        let items = vec![json!({"type":"TEXT"}); 100];
        assert_eq!(
            detail("MULTIPLY", &json!({"mergeMessage":items}))["itemCount"],
            100
        );
        let overflow = vec![json!({"type":"TEXT"}); 101];
        assert_eq!(
            detail("MULTIPLY", &json!({"mergeMessage":overflow}))["failureReason"],
            "too_many_merge_items"
        );
    }
}
