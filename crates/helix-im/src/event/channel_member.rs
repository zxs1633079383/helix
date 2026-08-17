//! Channel member 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造成员 roster 权威更新。
pub fn updated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:member-updated", data)
}

/// 构造成员群昵称定点更新，字段保持 Go 与 Angular 消费的 canonical casing。
pub fn nickname_updated(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:channel:member-nickname", data)
}
