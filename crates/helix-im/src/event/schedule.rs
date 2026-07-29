//! Schedule 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 构造定时消息创建事实。
pub fn created(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:schedule:created", data)
}

/// 构造定时消息取消终态。
pub fn canceled(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:schedule:canceled", data)
}
