//! Read 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造 viewer 的频道已读绝对态。
pub fn channel(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:read:channel", data)
}

/// 构造单条消息已读事实。
pub fn post(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:read", data)
}

/// 构造消息作者可见的 reader 列表。
pub fn post_readers(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:readers", data)
}
