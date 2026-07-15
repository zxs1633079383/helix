//! 单频道 hydration 的 wire channel -> render-ready dialog 投影。

use serde_json::Value;

use super::dialog_render::{as_i64, derive_last_post, truthy};

/// 把 `increment_channel` 的 wire camel channel 升成与本地 Scan 相同的 DialogProjection。
/// 直接用在手服务端事实解除 unknown/loading，不必让 UI 等待全表 channel Scan。
pub(crate) fn render_ready_dialog_wire_row(row: Value) -> Option<Value> {
    let Value::Object(mut obj) = row else {
        return None;
    };
    let id = obj.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let display_name = obj
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(id.as_str())
        .to_string();
    let channel_type = crate::channel_kind::channel_type_from_object(&obj).to_string();
    let channel_kind = crate::channel_kind::channel_kind(&channel_type);
    let unread_count = as_i64(obj.get("unreadCount")).unwrap_or(0);
    let mention_count = as_i64(obj.get("mentionCount")).unwrap_or(0);
    let mention_count_root = as_i64(obj.get("mentionCountRoot")).unwrap_or(0);
    let urgent_count = as_i64(obj.get("urgentCount")).unwrap_or(0);
    let event_seq = as_i64(obj.get("lastEventSeq")).unwrap_or(0);
    let delete_at = as_i64(obj.get("deleteAt")).unwrap_or(0);
    let top = truthy(obj.get("channelIsTop").or_else(|| obj.get("isTop")));
    let has_schedule_post = truthy(obj.get("hasSchedulePost"));
    let (last_message, last_post_urgent) = derive_last_post(obj.get("lastPost"));

    obj.insert("channelId".into(), Value::String(id));
    obj.insert("displayName".into(), Value::String(display_name));
    obj.insert("group".into(), Value::String(channel_kind.into()));
    obj.insert("channelType".into(), Value::String(channel_type));
    obj.insert("channelKind".into(), Value::String(channel_kind.into()));
    obj.insert("unread".into(), Value::from(unread_count));
    obj.insert("unreadCount".into(), Value::from(unread_count));
    obj.insert("mentionCount".into(), Value::from(mention_count));
    obj.insert("mentionCountRoot".into(), Value::from(mention_count_root));
    obj.insert(
        "mention".into(),
        Value::Bool(mention_count > 0 || mention_count_root > 0),
    );
    obj.insert("urgentCount".into(), Value::from(urgent_count));
    obj.insert(
        "urgent".into(),
        Value::Bool(last_post_urgent || urgent_count > 0 || truthy(obj.get("hasUrgentPost"))),
    );
    obj.insert("top".into(), Value::Bool(top));
    obj.insert("isTop".into(), Value::Bool(top));
    obj.insert("hasSchedulePost".into(), Value::Bool(has_schedule_post));
    obj.insert("eventSeq".into(), Value::from(event_seq));
    obj.insert("closed".into(), Value::Bool(delete_at > 0));
    obj.insert("loading".into(), Value::Bool(false));
    obj.insert("unknown".into(), Value::Bool(false));
    obj.insert("lastMessage".into(), last_message);
    if let Some(topic_count) = obj.get("topicMsgCount").cloned() {
        obj.insert("topicCount".into(), topic_count);
    }
    if let Some(last_message_at) = obj
        .get("lastPostAt")
        .or_else(|| obj.get("lastRootPostAt"))
        .cloned()
    {
        obj.insert("lastMessageAt".into(), last_message_at);
    }
    Some(Value::Object(obj))
}
