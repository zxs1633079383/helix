//! Timeline 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 从 durable message 行构造不含 Frame/Surface/action 派生字段的有界窗口终态。
#[allow(clippy::too_many_arguments)]
pub fn window(
    channel_id: &str,
    window_token: &str,
    state: &str,
    rows: Vec<Value>,
    has_older: bool,
    has_newer: bool,
    target_message_id: Option<&str>,
) -> Result<MessageV3Event, crate::ImError> {
    ready(serde_json::json!({
        "channelId": channel_id,
        "windowToken": window_token,
        "state": state,
        "messages": rows,
        "hasOlder": has_older,
        "hasNewer": has_newer,
        "targetMessageId": target_message_id,
    }))
}

/// 构造分页后的 persona-local 绝对窗口，不暴露 renderer mutation 或 action。
#[allow(clippy::too_many_arguments)]
pub fn page(
    channel_id: &str,
    window_token: &str,
    direction: &str,
    state: &str,
    rows: Vec<Value>,
    has_older: bool,
    has_newer: bool,
    anchor_post_id: Option<&str>,
) -> Result<MessageV3Event, crate::ImError> {
    update(serde_json::json!({
        "channelId": channel_id,
        "windowToken": window_token,
        "direction": direction,
        "state": state,
        "messages": rows,
        "hasOlder": has_older,
        "hasNewer": has_newer,
        "anchorPostId": anchor_post_id,
    }))
}

/// 构造离底窗口的新侧绝对态，并显式携带尚未消费的新消息数量。
#[allow(clippy::too_many_arguments)]
pub fn anchored_update(
    channel_id: &str,
    window_token: &str,
    state: &str,
    rows: Vec<Value>,
    has_older: bool,
    has_newer: bool,
    anchor_post_id: Option<&str>,
    newer_count: usize,
) -> Result<MessageV3Event, crate::ImError> {
    update(serde_json::json!({
        "channelId": channel_id,
        "windowToken": window_token,
        "direction": "newer",
        "state": state,
        "messages": rows,
        "hasOlder": has_older,
        "hasNewer": has_newer,
        "anchorPostId": anchor_post_id,
        "newerCount": newer_count,
    }))
}

/// 构造有界时间线就绪快照。
pub fn ready(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:timeline:ready", data)
}

/// 构造有界时间线增量。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:timeline:update", data)
}

/// 构造消息定位后的活动窗口。
pub fn located(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:timeline:located", data)
}

/// 构造回复分支快照。
pub fn thread(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:timeline:thread", data)
}
