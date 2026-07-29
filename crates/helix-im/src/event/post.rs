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

/// 从 posts_update authority 构造不含传输元数据的撤回事实。
pub fn revoke_from_authority(
    post: &Value,
    event_seq: u64,
) -> Result<MessageV3Event, crate::ImError> {
    revoke(canonical_revoke_data(post, Some(event_seq))?)
}

/// 从离线 type3 authority 构造同 identity 的最小撤回事实。
pub fn revoke_from_sync_authority(
    event: &crate::sync_session::EventEnvelope,
) -> Result<MessageV3Event, crate::ImError> {
    let id = event
        .msg_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| crate::ImError::Parse("sync revoke authority missing msgId".to_string()))?;
    revoke(serde_json::json!({
        "id": id,
        "channelId": event.channel_id.as_str(),
        "eventSeq": event.seq.0,
        "revoke": true,
    }))
}

/// 从 update_channel.lastPost authority 构造可持久化的业务 lastPost。
pub fn revoke_last_post_from_authority(post: &Value) -> Result<Value, crate::ImError> {
    canonical_revoke_data(post, None)
}

/// 把 snake/camel 入站别名收敛为单份 MessageV3 撤回字段。
fn canonical_revoke_data(post: &Value, event_seq: Option<u64>) -> Result<Value, crate::ImError> {
    let id = text_alias(post, &["id", "postId", "post_id"])
        .ok_or_else(|| crate::ImError::Parse("revoke authority missing id".to_string()))?;
    let channel_id = text_alias(post, &["channelId", "channel_id"])
        .ok_or_else(|| crate::ImError::Parse("revoke authority missing channelId".to_string()))?;
    let mut data = serde_json::json!({
        "id": id,
        "channelId": channel_id,
        "revoke": true,
    });
    if let Some(event_seq) = event_seq {
        data["eventSeq"] = serde_json::json!(event_seq);
    }
    for (canonical, aliases) in [
        ("temporaryId", &["temporaryId", "temporary_id"][..]),
        ("userId", &["userId", "user_id"][..]),
        ("type", &["type"][..]),
        ("message", &["message"][..]),
        ("createAt", &["createAt", "create_at"][..]),
        ("updateAt", &["updateAt", "update_at"][..]),
    ] {
        if let Some(value) = value_alias(post, aliases) {
            data[canonical] = value.clone();
        }
    }
    Ok(data)
}

/// 按顺序读取字符串别名，拒绝空 identity。
fn text_alias<'a>(value: &'a Value, aliases: &[&str]) -> Option<&'a str> {
    value_alias(value, aliases)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

/// 按协议优先级读取第一个存在的字段。
fn value_alias<'a>(value: &'a Value, aliases: &[&str]) -> Option<&'a Value> {
    aliases.iter().find_map(|key| value.get(*key))
}

/// 构造消息字段更新事实。
pub fn update(data: Value) -> Result<MessageV3Event, crate::ImError> {
    super::encode("im:post:update", data)
}
