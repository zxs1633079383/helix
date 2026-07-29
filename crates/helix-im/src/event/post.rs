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

/// 从已提交的 G-01 authority 构造无能力壳、无重复别名的 received 数据。
pub fn received_from_authority(
    event: &crate::sync_session::EventEnvelope,
) -> Result<MessageV3Event, crate::ImError> {
    received(serde_json::json!({
        "id": event.fields.id,
        "channelId": event.channel_id.as_str(),
        "temporaryId": event.fields.temporary_id,
        "eventSeq": event.seq.0,
        "userId": event.fields.user_id,
        "message": event.fields.message,
        "type": event.fields.msg_type,
        "createAt": event.fields.create_at,
        "sendStatus": "sent",
    }))
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
