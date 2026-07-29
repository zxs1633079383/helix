//! Timeline 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

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
