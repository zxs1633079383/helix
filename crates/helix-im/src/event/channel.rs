//! Channel 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造频道创建后的权威快照。
pub fn created(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:created", data)
}

/// 构造频道及当前 viewer 绝对态更新。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:update", data)
}

/// 构造频道关闭终态。
pub fn closed(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:closed", data)
}

/// 构造当前 viewer 已离开频道终态。
pub fn left(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:left", data)
}

/// 构造话题频道创建事实。
pub fn topic_created(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:topic-created", data)
}

/// 构造频道设置权威更新。
pub fn settings_updated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:settings-updated", data)
}
