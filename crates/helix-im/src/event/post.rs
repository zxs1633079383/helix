//! Post 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造本地发送中事实。
pub fn sending(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:sending", data)
}

/// 构造服务端已接收消息事实。
pub fn received(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:received", data)
}

/// 构造发送失败事实。
pub fn send_failed(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:send-failed", data)
}

/// 构造消息撤回事实。
pub fn revoke(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:revoke", data)
}

/// 构造消息字段更新事实。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:update", data)
}
