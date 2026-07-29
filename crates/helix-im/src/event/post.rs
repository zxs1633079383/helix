//! Post 领域 MessageV3 事件。

use super::MessageV3Event;
use serde_json::Value;

/// 保存一次 authority 投影生成的 post 终态与 channel lastPost。
pub struct AuthorityProjection {
    pub received_data: Value,
    pub last_post: Value,
}

/// 构造本地发送中事实。
pub fn sending(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:sending", data)
}

/// 从本地待发送 body 构造只含业务字段的 optimistic 事实。
pub fn sending_from_local_body(
    channel_id: &str,
    temporary_id: &str,
    user_id: &str,
    body: &Value,
) -> Result<MessageV3Event, crate::ImError> {
    let mut data = serde_json::json!({
        "channelId": channel_id,
        "temporaryId": temporary_id,
        "message": body.get("message").and_then(Value::as_str).unwrap_or(""),
        "type": body.get("type").and_then(Value::as_str).unwrap_or("TEXT"),
        "createAt": body.get("createAt").and_then(Value::as_i64).unwrap_or_default(),
        "sendStatus": "sending",
    });
    if !user_id.is_empty() {
        data["userId"] = serde_json::json!(user_id);
    }
    for key in ["replyId", "replyRootId", "replyFirstLevelId"] {
        if let Some(value) = body
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            data[key] = serde_json::json!(value);
        }
    }
    sending(data)
}

/// 构造服务端已接收消息事实。
pub fn received(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:received", data)
}

/// 从 authority 一次解析 props，并生成 post 与 channel 共用的真实结构。
pub fn authority_projection(
    event: &crate::sync_session::EventEnvelope,
) -> Result<AuthorityProjection, crate::ImError> {
    let mut received_data = serde_json::json!({
        "id": event.fields.id,
        "channelId": event.channel_id.as_str(),
        "temporaryId": event.fields.temporary_id,
        "eventSeq": event.seq.0,
        "userId": event.fields.user_id,
        "message": event.fields.message,
        "type": event.fields.msg_type,
        "createAt": event.fields.create_at,
        "sendStatus": "sent",
    });
    let mut last_post = serde_json::json!({
        "id": event.fields.id,
        "userId": event.fields.user_id,
        "message": event.fields.message,
        "createAt": event.fields.create_at,
    });
    if event.fields.msg_type != "TEXT" {
        last_post["type"] = serde_json::json!(event.fields.msg_type.as_str());
    }
    let mut props = if event.fields.props.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str::<Value>(&event.fields.props)
            .map_err(|error| crate::ImError::Parse(format!("post props: {error}")))?
    };
    if let Some(object) = props.as_object_mut() {
        object.remove("channel_event_seq");
    }
    if !props.as_object().is_some_and(serde_json::Map::is_empty) && !props.is_null() {
        received_data["props"] = props.clone();
        last_post["props"] = props;
    }
    Ok(AuthorityProjection {
        received_data,
        last_post,
    })
}

/// 构造发送失败事实。
pub fn send_failed(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:send-failed", data)
}

/// 构造按 temporaryId 锚定的本地发送失败事实。
pub fn send_failed_for_identity(
    channel_id: &str,
    temporary_id: &str,
) -> Result<MessageV3Event, crate::ImError> {
    send_failed(serde_json::json!({
        "channelId": channel_id,
        "temporaryId": temporary_id,
        "sendStatus": "failed",
    }))
}

/// 构造消息撤回事实。
pub fn revoke(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:revoke", data)
}

/// 构造消息字段更新事实。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:update", data)
}
